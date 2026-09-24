#include <stdio.h>
#include <string.h>
#include <inttypes.h>

#include "driver/gpio.h"
#include "driver/rmt_rx.h"
#include "esp_err.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include "hal/gpio_ll.h"
#include "soc/gpio_num.h"

#define DALI_RX_GPIO 5
#define DALI_TX_GPIO 14

#define RMT_RESOLUTION_HZ 1000000

#define RMT_MEM_BLOCK_SYMBOLS 96

#define MAX_SYMBOLS 256
#define NUM_BUFFERS 4
#define QUEUE_DEPTH 16

#define GLITCH_FILTER_NS 2000

#define IDLE_END_NS 2000000
#define IDLE_END_US (IDLE_END_NS / 1000)

static const char *TAG = "arbiter";

typedef struct {
    int64_t t_end_us;
    uint32_t seq;
    uint16_t num_symbols;
    uint8_t buf_index;
} frame_msg_t;

static rmt_symbol_word_t s_bufs[NUM_BUFFERS][MAX_SYMBOLS];
static rmt_channel_handle_t s_rx_chan;
static QueueHandle_t s_queue;
static rmt_receive_config_t s_recv_cfg;

static volatile uint32_t s_seq;
static volatile uint32_t s_dropped;
static volatile uint32_t s_rearm_err;
static volatile uint8_t s_cur_buf;

static IRAM_ATTR bool on_recv_done(rmt_channel_handle_t chan,
                                   const rmt_rx_done_event_data_t *edata,
                                   void *user_ctx)
{
    (void)chan;
    (void)user_ctx;
    BaseType_t high_task_wakeup = pdFALSE;

    frame_msg_t msg = {
        .t_end_us = esp_timer_get_time(),
        .seq = s_seq++,
        .num_symbols = (uint16_t)edata->num_symbols,
        .buf_index = s_cur_buf,
    };

    s_cur_buf = (uint8_t)((s_cur_buf + 1) % NUM_BUFFERS);
    if (rmt_receive(s_rx_chan, s_bufs[s_cur_buf], sizeof(s_bufs[s_cur_buf]),
                    &s_recv_cfg) != ESP_OK) {
        s_rearm_err++;
    }

    if (xQueueSendFromISR(s_queue, &msg, &high_task_wakeup) != pdTRUE) {
        s_dropped++;
    }
    return high_task_wakeup == pdTRUE;
}

static void park_tx_recessive(void)
{
    gpio_config_t cfg = {
        .pin_bit_mask = 1ULL << DALI_TX_GPIO,
        .mode = GPIO_MODE_OUTPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_ENABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    ESP_ERROR_CHECK(gpio_config(&cfg));
    ESP_ERROR_CHECK(gpio_set_level(DALI_TX_GPIO, 0));
}

static void scan_pins(uint32_t ms)
{
    static const int candidates[] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
                                     14, 15, 16, 17, 18, 19, 20, 21, 22, 23};
    const size_t n = sizeof(candidates) / sizeof(candidates[0]);
    int last[sizeof(candidates) / sizeof(candidates[0])];
    uint32_t edges[sizeof(candidates) / sizeof(candidates[0])];

    for (size_t i = 0; i < n; i++) {
        if (candidates[i] == DALI_TX_GPIO) {
            continue;
        }
        gpio_config_t cfg = {
            .pin_bit_mask = 1ULL << candidates[i],
            .mode = GPIO_MODE_INPUT,
            .pull_up_en = GPIO_PULLUP_DISABLE,
            .pull_down_en = GPIO_PULLDOWN_DISABLE,
            .intr_type = GPIO_INTR_DISABLE,
        };
        gpio_config(&cfg);
        last[i] = gpio_get_level(candidates[i]);
        edges[i] = 0;
    }

    int64_t deadline = esp_timer_get_time() + (int64_t)ms * 1000;
    while (esp_timer_get_time() < deadline) {
        for (size_t i = 0; i < n; i++) {
            if (candidates[i] == DALI_TX_GPIO) {
                continue;
            }
            int lvl = gpio_ll_get_level(&GPIO, candidates[i]);
            if (lvl != last[i]) {
                last[i] = lvl;
                edges[i]++;
            }
        }
    }

    printf("# pinscan ms=%" PRIu32 "\n", ms);
    for (size_t i = 0; i < n; i++) {
        if (candidates[i] == DALI_TX_GPIO) {
            printf("# pin %2d parked-tx\n", candidates[i]);
            continue;
        }
        printf("# pin %2d level=%d edges=%" PRIu32 "\n", candidates[i], last[i], edges[i]);
    }
    fflush(stdout);
}

static void printer_task(void *arg)
{
    (void)arg;
    static char line[MAX_SYMBOLS * 8 + 64];
    frame_msg_t msg;

    for (;;) {
        if (xQueueReceive(s_queue, &msg, portMAX_DELAY) != pdTRUE) {
            continue;
        }
        const rmt_symbol_word_t *sym = s_bufs[msg.buf_index];

        int pos = 0;
        uint32_t total = 0;
        int npulse = 0;
        int first_level = msg.num_symbols ? sym[0].level0 : -1;

        pos += snprintf(line + pos, sizeof(line) - pos, "E %" PRIu32 " %" PRId64 " %d",
                        msg.seq, msg.t_end_us, first_level);
        for (uint16_t i = 0; i < msg.num_symbols && pos < (int)sizeof(line) - 24; i++) {
            pos += snprintf(line + pos, sizeof(line) - pos, " %u", (unsigned)sym[i].duration0);
            total += sym[i].duration0;
            npulse++;
            if (sym[i].duration1 == 0 && i + 1 == msg.num_symbols) {
                break;
            }
            pos += snprintf(line + pos, sizeof(line) - pos, " %u", (unsigned)sym[i].duration1);
            total += sym[i].duration1;
            npulse++;
        }
        (void)total;
        (void)npulse;
        printf("%s\n", line);
    }
}

static void stats_task(void *arg)
{
    (void)arg;
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(5000));
        printf("# stats seq=%" PRIu32 " dropped=%" PRIu32 " rearm_err=%" PRIu32
               " t=%" PRId64 "\n",
               s_seq, s_dropped, s_rearm_err, esp_timer_get_time());
        fflush(stdout);
    }
}

void app_main(void)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    esp_log_level_set("*", ESP_LOG_WARN);

    park_tx_recessive();

    printf("\n# dali-arbiter build=%s %s\n", __DATE__, __TIME__);
    printf("# rx_gpio=%d tx_gpio=%d(parked) res_hz=%d glitch_ns=%d idle_end_us=%d\n",
           DALI_RX_GPIO, DALI_TX_GPIO, RMT_RESOLUTION_HZ, GLITCH_FILTER_NS, IDLE_END_US);
    fflush(stdout);

    scan_pins(1000);

    s_queue = xQueueCreate(QUEUE_DEPTH, sizeof(frame_msg_t));
    configASSERT(s_queue);

    rmt_rx_channel_config_t rx_cfg = {
        .gpio_num = DALI_RX_GPIO,
        .clk_src = RMT_CLK_SRC_DEFAULT,
        .resolution_hz = RMT_RESOLUTION_HZ,
        .mem_block_symbols = RMT_MEM_BLOCK_SYMBOLS,
    };
    ESP_ERROR_CHECK(rmt_new_rx_channel(&rx_cfg, &s_rx_chan));

    rmt_rx_event_callbacks_t cbs = {.on_recv_done = on_recv_done};
    ESP_ERROR_CHECK(rmt_rx_register_event_callbacks(s_rx_chan, &cbs, NULL));

    s_recv_cfg.signal_range_min_ns = GLITCH_FILTER_NS;
    s_recv_cfg.signal_range_max_ns = IDLE_END_NS;

    ESP_ERROR_CHECK(rmt_enable(s_rx_chan));
    ESP_ERROR_CHECK(rmt_receive(s_rx_chan, s_bufs[0], sizeof(s_bufs[0]), &s_recv_cfg));

    xTaskCreate(printer_task, "printer", 4096, NULL, 5, NULL);
    xTaskCreate(stats_task, "stats", 3072, NULL, 3, NULL);

    ESP_LOGW(TAG, "capturing");
    printf("# ready\n");
    fflush(stdout);
}

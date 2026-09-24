# Runtime-модули

Указатель модулей контроллера: что каждый делает, где живёт и кто за что отвечает.
Каждый модуль описан в своём файле; здесь — только карта и общие правила.

Границы: потоки, стеки и ядра — [`../../architecture/02-runtime-and-threading.md`](../../architecture/02-runtime-and-threading.md);
шина и ёмкости инбоксов — [`../../architecture/03-bus-and-backpressure.md`](../../architecture/03-bus-and-backpressure.md);
что построено — [`../status.md`](../status.md).

## Модули

| Модуль | Крейт | Отвечает за |
|---|---|---|
| [`api-gateway`](api-gateway/README.md) | `dali2rust-api` | HTTP: валидация, публикация команд, ответ; ни одной мутации мимо шины |
| [`registry`](registry/README.md) ([read-port](registry/read-port.md)) | `dali2rust-registry-runtime` | Единственный писатель состояния: конфигурация, доказательства, runtime, персист |
| [`operation-tracker`](operation-tracker/README.md) | `dali2rust-operations-runtime` | Жизненный цикл долгих операций и их read-модель |
| [`apply-orchestrator`](apply-orchestrator/README.md) | `dali2rust-operations-runtime` | Развёртка и пейсинг массового применения: матрицы групп и сцен, политики |
| [`dali-worker`](dali-worker/README.md) | `dali2rust-dali-runtime` | Единственный владелец провода: исполнение семантических команд, приоритет, транзакции |
| [`sniffer-translator`](sniffer-translator/README.md) | `dali2rust-fanout-runtime` | Сырые чужие кадры → типизированные наблюдения и события входа |
| [`state-fanout`](state-fanout/README.md) | `dali2rust-fanout-runtime` | Единственный издатель runtime-коммитов реестра |
| [`hcl-scheduler`](hcl-scheduler/README.md) | `dali2rust-hcl-runtime` | Расписания цветовой температуры и уровня на группы и broadcast |
| [`poller`](poller/README.md) | `dali2rust-poller-runtime` | Фоновое чтение устройств в пределах бюджета провода |
| [`mqtt-home-assistant`](mqtt-home-assistant/README.md) | `dali2rust-mqtt-runtime` | Мост Home Assistant: discovery, состояние, команды |
| [`websocket`](websocket/README.md) | `dali2rust-ws-runtime` | Живая лента для web UI поверх той же JSON-формы, что REST |
| [`stats`](stats/README.md) | `dali2rust-api` + композиция | Read-модель `/api/v1/stats` поверх счётчиков |
| [`display`](display/README.md) | `dali2rust-display-runtime` | Сводный экран OLED |
| [`input-devices`](input-devices/README.md) | без своего крейта (`dev103` в `DaliWorker`) | Устройства ввода IEC 62386-103: коммиссионинг, конфигурация, события, индикация |
| [`rules-engine`](rules-engine/README.md) ([язык](rules-engine/operations.md)) | `dali2rust-rules-model`, `-lang`, `-runtime` | Локальная автоматика: события → действия |
| [`redundancy`](redundancy/README.md) | `dali2rust-redundancy-runtime` | Пара контроллеров на одном сегменте: арбитраж, лиза, репликация |
| Обновление прошивки | `dali2rust-ota-runtime` | Загрузка образа по сети в неактивный слот ([`ADR-024`](../../architecture/decisions/ADR-024-ota-over-ethernet.md), [REST](../rest-api/resources/firmware.md)) |
| [`cluster`](cluster/README.md) | — | Запланирован (`I6`) |
| [`dali-proxy`](dali-proxy/README.md) | — | Запланирован (`I7`) |

`dali2rust-adapters` модулем не является: он только подписывает воркеров на шину,
спавнит их и связывает порты
([`../../architecture/01-overview.md`](../../architecture/01-overview.md)). Кто
единственный владелец чего — [`../glossary-and-invariants.md`](../glossary-and-invariants.md)
§Инварианты; поведение модулей на пассивном контроллере —
[`redundancy`](redundancy/README.md) §Пассивный контроллер.

## Один адаптер сегодня

Сегодня собран один адаптер
([`../../architecture/01-overview.md`](../../architecture/01-overview.md) §Composition);
хостовый стек BDD держит два логических адаптера на одном исполнителе. Целевая
композиция — [`../roadmap.md`](../roadmap.md) §X2.

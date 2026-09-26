# Ресурс: Settings — Home Assistant

Настройки моста MQTT / Home Assistant и ручная повторная публикация discovery.

**Границы:** поведение моста (сессии, discovery, state, команды) —
[`../../runtime-modules/mqtt-home-assistant/README.md`](../../runtime-modules/mqtt-home-assistant/README.md);
сводка подключения — [`controller.md`](controller.md). BDD —
[`settings_home_assistant`](../../../../tests/dali2rust-bdd/features/settings_home_assistant/),
[`mqtt_home_assistant`](../../../../tests/dali2rust-bdd/features/mqtt_home_assistant/).

## Маршруты

| Метод | Путь | Ответ |
|---|---|---|
| `GET` | `/api/v1/settings/home-assistant` | `200` |
| `PATCH` | `/api/v1/settings/home-assistant` | `200` — применённые настройки |
| `POST` | `/api/v1/settings/home-assistant/discovery-publish` | `202` `ha_discovery_publish` |

## Поля

- `enabled`, `broker_host`, `broker_port`, `broker_username`, `publish_qos` (`0` или
  `1`), `retain_state`, `retain_discovery`.
- `broker_password` — **только на запись**: ни один путь чтения пароль не возвращает;
  вместо него read-only `broker_password_set`. На флеше пароль лежит в слайсе настроек
  открытым текстом — флеш не шифруется, — и экспорт слайса пиру несёт его так же
  ([ADR-018](../../../architecture/decisions/ADR-018-controller-redundancy.md)). Это
  ожидаемое поведение: контроллер рассчитан на доверенную LAN, и пароль закрыт только от
  путей чтения REST.
- `broker_url_view` — read-only строка для людей (что контроллер будет набирать), а не
  разбираемый URL; пустая, пока брокер не задан.
- `discovery_prefix`, `state_topic_prefix` — префиксы топиков.
- `controller_id` — корень `unique_id` сущностей в Home Assistant и `node_id` топиков
  discovery; символы вне безопасного для топика набора — `422 invalid_value` (обход
  ISSUE-145 в [`../../known-issues.md`](../../known-issues.md)).
- `expose_input_devices` — глобальная половина гейта выставления устройств ввода
  (вторая половина — `ha_expose` устройства в [`input-devices.md`](input-devices.md)).

По умолчанию мост выключен и брокер не задан — сам контроллер никуда не подключается;
порт 1883, QoS 1, retain включён, префиксы — `homeassistant` и `dali` (дефолты самого
Home Assistant), устройства ввода выставляются. `controller_id` по умолчанию выводится
из трёх последних байт MAC Ethernet (`dali-xxxxxx`), а без MAC — постоянное
`dali-controller`, по которому в топике видно, что контроллер своего адреса не узнал.

## `PATCH`

Запись настроек не помещается в один кадр шины и делится по группам полей: один
`PATCH` публикует до **четырёх** команд (общие настройки, учётные данные, топики,
`controller_id`) и ждёт их подтверждений под **одним** дедлайном, затем
read-after-write. Частичная публикация — `503 partial_apply`, повтор безопасен.

- Длины проверяются на входе по ширине полей команд: строка, которую шина обрезала бы
  молча (пароль, обрезанный до ширины поля, не аутентифицирует ничего, и брокер не
  называет причину), — `422 invalid_value`. Хост и учётные данные можно очистить
  пустой строкой; префиксы и `controller_id` пустыми быть не могут.
- `broker_password_set` и `broker_url_view` в теле — `422 unsupported_field` (клиент
  отправил обратно прочитанное); неизвестное поле — `400 unknown_field`; порт `0` или
  QoS > 1 — `422 invalid_value`.
- Изменение брокера, идентичности или префиксов мост применяет новой сессией
  ([`mqtt-home-assistant`](../../runtime-modules/mqtt-home-assistant/README.md) §Сессия).

## `POST …/discovery-publish`

Без тела. Ручной переанонс: discovery всех выставленных сущностей с уборкой
устаревших; чем он отличается от переанонса новой сессии —
[`mqtt-home-assistant`](../../runtime-modules/mqtt-home-assistant/README.md) §Сессия.
Прогон темпирован, поэтому ответ — `202` + `operation_id`, а итог (сколько сущностей
опубликовано и сколько не удалось) — в операции.

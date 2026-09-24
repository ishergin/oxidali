# Реестр ID-префиксов BDD

Единственная точка выделения префиксов `@id` для сценариев
`tests/dali2rust-bdd/features/**`.

Границы: здесь только префиксы. Формат тегов и правила сценариев —
[`05-testing-and-bdd.md`](../../architecture/05-testing-and-bdd.md); сами сценарии и их
номера — в дереве `features/`.

## Правила

- Префикс оканчивается дефисом; внутри могут быть ещё дефисы (`SET-HA-`).
- `scripts/verify_bdd_ids.sh` берёт префиксы из первой колонки таблицы ниже, поэтому
  префикс существует, только пока он в ней.
- Новый домен регистрирует префикс здесь до первого сценария. Новый сценарий берёт
  свободный номер под префиксом своего домена; номер удалённого сценария не
  переиспользуется.
- Список занятых номеров даёт само дерево:
  `grep -rhoE '@id:[A-Za-z0-9-]+' tests/dali2rust-bdd/features | sort -u`.

## Префиксы

| Префикс | Каталог `features/` | Домен |
|---|---|---|
| `ADP-` | `adapters/` | DALI-адаптеры |
| `BUS-` | `contracts/` | Доставка HTTP → шина → провод |
| `CFG-` | `config_transfer/` | Слайсы конфигурации: манифест, экспорт, импорт |
| `COMM-` | `commissioning/` | Коммиссионинг control gear |
| `CONT-` | `contracts/` | Соответствие HTTP-команды сообщению шины и кадру DALI |
| `DALI-` | `diagnostic/`, `system/` | Диагностический путь `/api/v1/dali/*`; таймаут подтверждения и параллельные команды |
| `DIAG-` | `diagnostic/` | `/api/v1/diagnostics` |
| `GRP-` | `groups/` | Группы |
| `HCL-` | `hcl/` | Расписания HCL и планировщик |
| `INP-` | `input_devices/` | Устройства ввода IEC 62386-103 |
| `MQTT-` | `mqtt_home_assistant/` | Мост MQTT / Home Assistant |
| `OP-` | `operations/` | Операции |
| `PD-` | `physical_devices/` | Физические устройства |
| `PERS-` | `persistence/` | Персистентность слайсов |
| `POL-` | `poller/` | Поллер |
| `POLICY-` | `policies/` | Политики `systemFailureLevel` / `powerOnLevel` |
| `RED-` | `redundancy/` | Резервирование контроллера |
| `REG-` | `groups/`, `scenes/` | Реестр: desired против applied у групп и сцен |
| `RULE-` | `rules/` | Движок правил |
| `SCN-` | `scenes/` | Сцены |
| `SET-DALI-` | `settings_dali/` | Настройки DALI |
| `SET-HA-` | `settings_home_assistant/` | Настройки Home Assistant |
| `SET-POL-` | `settings_poller/` | Настройки поллера |
| `STATS-` | `stats/` | Статистика `/api/v1/stats` |
| `SYS-` | `system/` | Сквозное поведение стека: загрузка, здоровье, композиция, интеграция |
| `VL-` | `virtual_lamps/` | Виртуальные лампы |
| `WEB-` | `web_ui/` | Раздача встроенного веб-интерфейса |
| `WS-` | `websocket/` | WebSocket |

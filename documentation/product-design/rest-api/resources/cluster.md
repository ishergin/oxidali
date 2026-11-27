# Ресурс: Cluster

Параметры узла кластера: участвует ли контроллер в кластере, имя кластера, номер узла,
порт, провод DALI и что узел пересылает и принимает.

**Границы:** что делает кластер и почему —
[`../../runtime-modules/cluster/README.md`](../../runtime-modules/cluster/README.md) и
[ADR-035](../../../architecture/decisions/ADR-035-cluster-of-installations-over-udp.md);
узел и канал как термины —
[`../../glossary-and-invariants.md`](../../glossary-and-invariants.md). BDD —
[`cluster`](../../../../tests/dali2rust-bdd/features/cluster/).

## Маршруты

| Метод | Путь | Ответ |
|---|---|---|
| `GET` | `/api/v1/cluster` | `200` |
| `PATCH` | `/api/v1/cluster` | `200` — применённые настройки |

`PATCH` — merge-patch с read-after-write. Правила, связывающие два поля, проверяются на
сохранённых настройках вместе с патчем; отказанный патч не меняет ничего. Настройки
персистятся своим слайсом `cluster_settings` и входят в конфигурацию, которую забирает
резервная плата: пара делит номер узла.

## Поля

| Поле | По умолчанию | Смысл |
|---|---|---|
| `enabled` | `false` | Участвует ли узел в кластере. |
| `cluster_name` | `""` | До 64 байт UTF-8; без имени кластер не включается. |
| `node_number` | `0` | `0..7`. |
| `port` | `47810` | UDP-порт кластера, `1..65535`. |
| `wire_same_as` | `null` | `null` — свой провод; номер узла `0..7` — тот же провод, что у этого узла. |
| `forward_operator` | `true` | Пересылать команды оператора. |
| `forward_home_assistant` | `true` | Пересылать команды Home Assistant. |
| `forward_hcl` | `false` | Пересылать уставки HCL. |
| `forward_foreign_master` | `false` | Пересылать кадры чужого мастера со сниффера. |
| `accept_all_lamps` | `false` | Исполнять входящие команды «всем». |
| `send_all_lamps` | `false` | Отправлять свои команды «всем» в кластер. |
| `accept_single_lamps` | `false` | Исполнять входящие команды одиночным лампам этого узла. |
| `wire_protection` | `true` | Защита от петли на общем проводе. |
| `read_only` | — | Только чтение: `true` на резервной плате. |
| `block` | — | `{first, last}` — блок номеров записей узла: `node_number·16 … node_number·16+15`. |

Источника «правила» нет: команды правил пересылаются только со стадией правил.

## Ошибки

| Случай | Ответ |
|---|---|
| Включение без имени или пустое имя у включённого кластера | `409 conflict`, `message: cluster_name_required` |
| `wire_same_as` называет этот же узел | `409 conflict`, `message: wire_names_this_node` |
| Защита выключена, чужой мастер пересылается и провод общий | `409 conflict`, `message: wire_protection_required` |
| Резервная плата | `409 controller_standby`, `Retry-After: 1`, `X-Dali2rust-Role: standby` |
| Номер, порт или провод вне диапазона, имя длиннее 64 байт, значение неверного типа | `422 invalid_value` |
| `read_only` или `block` в теле | `422 unsupported_field` |
| Неизвестное поле | `400 unknown_field` |

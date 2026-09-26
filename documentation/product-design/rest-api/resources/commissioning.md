# Ресурс: Commissioning

Сервисные процедуры control gear, которые меняют адресацию или переносят логическую
роль на другой прибор: identify, смена адреса, замена прибора и типизированные
expert-шаги IEC.

**Границы:** метаданные и атрибуты прибора — [`physical-devices.md`](physical-devices.md)
(его `PATCH` адрес не меняет никогда); коммиссионинг устройств ввода Part 103 —
[`input-devices.md`](input-devices.md); сырые кадры —
[`diagnostic-dali.md`](diagnostic-dali.md), не часть этого пути; результаты —
[`operations.md`](operations.md). BDD —
[`commissioning`](../../../../tests/dali2rust-bdd/features/commissioning/).

Handler всегда публикует типизированную команду commissioning, в том числе для
expert-шагов; последовательность IEC строит DALI worker
([09 §Product path](../../../architecture/09-dali-protocol-rules.md#product-path-and-diagnostic-path)).

## Маршруты

| Метод | Путь (`/api/v1/adapters/{adapter_id}/commissioning/…`) | Ответ |
|---|---|---|
| `POST` | `identify` | `202` `commissioning_identify` |
| `POST` | `address-changes` | `202` `commissioning_address_change` |
| `POST` | `replacements` | `202` `commissioning_replace_device` |
| `POST` | `steps/{step}` | `200` (синхронное подтверждение) |

## Исключение на адаптер

Пока на адаптере идёт любая из трёх commissioning-операций, **любой** маршрут этого
ресурса на том же адаптере отвечает `409 conflict` — identify не может начаться под
идущей сменой адреса. Discovery и прочие операции правило не затрагивает. Процедуры
провод не уступают, так что команда оператора в это время может честно получить `504`
([`ADR-013`](../../../architecture/decisions/ADR-013-wire-priority-and-yield-granularity.md)).

## `POST identify`

Тело — только `{"short_address": N}`; любой другой ключ — `422 unsupported_field`.
Прибор должен быть известен реестру (`404 not_found`), адрес вне `0..63` — `422
invalid_value`.

- Worker шлёт одну пару `IDENTIFY DEVICE` и ничего больше; окно около 10 с принадлежит
  прибору, поэтому длительности в запросе нет, а индикация не обязана быть светом
  ([09 §Faults and identification](../../../architecture/09-dali-protocol-rules.md#faults-and-identification)).
- Результат операции: `{short_address, identify_mechanism}`. Живой механизм —
  `identify_device`; `blink_recall_max_min` остаётся в перечислении только как подпись
  старых записей.

## `POST address-changes`

Тело: `short_address`, `new_short_address`, `verify_after_program` (по умолчанию
`true`).

- Ошибки: адрес вне `0..63` или адреса равны — `422 invalid_value`; исходного прибора
  нет — `404 not_found`; целевой адрес занят другим прибором — `409 conflict`.
- Операнд доказывается до `SET SHORT ADDRESS`
  ([`ADR-027`](../../../architecture/decisions/ADR-027-dtr-operand-proof-and-readback-outcomes.md)),
  проверка идёт по новому адресу
  ([09 §Addressing control gear](../../../architecture/09-dali-protocol-rules.md#addressing-control-gear)).
- При успехе реестр **сам** переносит запись на новый адрес: старый адрес сразу
  `404`, привязанная виртуальная лампа остаётся той же сущностью с тем же HA
  `unique_id`. Результат: `{old_short_address, new_short_address}`.
- Неподтверждённая проверка — операция `failed` с `verify_failed` /
  `verify_unanswered` / `verify_contended` ([`../contracts/error-dto.md`](../contracts/error-dto.md)).

## `POST replacements`

Тело: `failed_short_address`, `replacement_short_address` (уже известный прибор того же
адаптера) и `restore` — флаги `metadata_and_overrides`, `attributes`, `groups`,
`scenes` (все по умолчанию `true`, хотя бы один обязан остаться `true`).

- Ошибки: любого из приборов нет — `404`; адреса вне диапазона, равны или все флаги
  `false` — `422 invalid_value`.
- Worker переадресует заменитель на адрес отказавшего прибора и восстанавливает на нём
  выбранные слайсы по порядку: метаданные и override'ы, записываемые атрибуты,
  applied-членство в группах, запрограммированные сцены. Сырые банки памяти не
  копируются.
- **Старый прибор должен быть снят с шины заранее**: пока он отвечает на сохраняемом
  адресе, операция завершается `verify_failed`.
- Логическая привязка и HA-идентичность сохраняются. Результат —
  `{failed_short_address, replacement_short_address, restored{…}}`, где флаги —
  реально восстановленные слайсы.

## `POST steps/{step}` — expert-шаги

Типизированные примитивы IEC для ручного коммиссионинга. Операции не создают.

| `step` | Тело | Добавки в ответе |
|---|---|---|
| `initialise` | `{scope: all \| unaddressed \| short, short_address?}` | — |
| `randomise` | `{}` | — |
| `search-address` | `{search_address}` (24 бита) | — |
| `compare` | `{}` | `match` |
| `program-short-address` | `{short_address}` | — |
| `verify-short-address` | `{short_address}` | `match` |
| `query-short-address` | `{}` | `short_address`, `answer` |
| `withdraw`, `terminate` | `{}` | — |

- Ответ — `success`, `backward_frame` и добавки шага; отсутствующее поле значит «к
  шагу не относится». Отказ исполнения тоже приходит `200` с `success: false`, и тогда
  в ответе есть `error_code` и `message`, если исполнитель назвал причину (например,
  `adapter_disabled`), — чтобы собственный выключатель оператора не читался как мёртвый
  прибор.
- **Нарушающий кадр — это ответ**
  ([09 §Reading answers](../../../architecture/09-dali-protocol-rules.md#reading-answers)):
  для `compare` и `verify-short-address` он даёт `match: true`.
- `answer` у `query-short-address` различает четыре исхода, три из которых дают
  `short_address: null`: `address` (ответил один прибор, адрес в `short_address`),
  `unaddressed` (ответил MASK — у прибора нет адреса; декодировать MASK как адрес 63
  нельзя), `multiple` (нарушающий кадр — несколько приборов), `none` (тишина; так же
  выглядит и потерянный запрос).
- Ошибки: неизвестный `step` — `404 not_found`; неизвестный `scope` — `422
  invalid_enum`; адрес вне диапазона или `search_address` шире 24 бит — `422
  invalid_value`; таймаут подтверждения — `504`.
- Шаги не сверяют реестр: ручные изменения адресации закрепляются последующим
  discovery или высокоуровневой процедурой.

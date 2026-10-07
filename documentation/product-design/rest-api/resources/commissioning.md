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

Пока на адаптере идёт любая commissioning-операция — три операции этого ресурса,
`commission` или `identify` устройств ввода Part 103
([`input-devices.md`](input-devices.md)), — **любой** маршрут этого ресурса и оба этих
маршрута устройств ввода на том же адаптере отвечают `409 conflict`, не опубликовав
ничего: identify не может начаться под идущей сменой адреса, а адресация устройств
ввода — под адресацией control gear. Discovery, скан устройств ввода и прочие операции
правило не затрагивает. Процедуры провод не уступают, так что команда оператора в это
время может честно получить `504`
([`ADR-013`](../../../architecture/decisions/ADR-013-wire-priority-and-yield-granularity.md)).

## `POST identify`

Тело — только `{"short_address": N}`; любой другой ключ — `400 unknown_field`.
Прибор должен быть известен реестру (`404 not_found`), адрес вне `0..63` — `422
invalid_value`.

- Worker шлёт одну пару `IDENTIFY DEVICE` и ничего больше; окно около 10 с принадлежит
  прибору, поэтому длительности в запросе нет, а индикация не обязана быть светом
  ([09 §Faults and identification](../../../architecture/09-dali-protocol-rules.md#faults-and-identification)).
- Результат операции: `{short_address, identify_mechanism: "identify_device"}`.

## `POST address-changes`

Тело: `short_address`, `new_short_address`, `verify_after_program` (по умолчанию
`true`).

- Ошибки: неизвестный ключ тела — `400 unknown_field`; адрес вне `0..63` или адреса
  равны — `422 invalid_value`; исходного прибора нет — `404 not_found`; целевой адрес
  занят другим прибором — `409 conflict`.
- Операнд доказывается до `SET SHORT ADDRESS`
  ([`ADR-027`](../../../architecture/decisions/ADR-027-dtr-operand-proof-and-readback-outcomes.md)),
  проверка идёт по новому адресу
  ([09 §Addressing control gear](../../../architecture/09-dali-protocol-rules.md#addressing-control-gear)).
- При успехе реестр **сам** переносит запись на новый адрес: старый адрес сразу
  `404`, привязанная виртуальная лампа остаётся той же сущностью с тем же HA
  `unique_id`. Результат: `{old_short_address, new_short_address}`. Реестр публикует
  `PhysicalDeviceChangedEvent` обоих адресов и `VirtualLampChangedEvent` перенесённой
  лампы.
- Неподтверждённая проверка — операция `failed` с `verify_failed` /
  `verify_unanswered` / `verify_contended` ([`../contracts/error-dto.md`](../contracts/error-dto.md)).

## `POST replacements`

Тело: `failed_short_address`, `replacement_short_address` (уже известный прибор того же
адаптера) и необязательный `restore` с единственным флагом `metadata_and_overrides`
(по умолчанию `true`).

- Ошибки: неизвестный ключ тела (и в `restore`) — `400 unknown_field`; любого из
  приборов нет — `404`; адреса вне диапазона или равны — `422 invalid_value`; оба
  адреса привязаны к лампам — `409 conflict` с `message =
  replacement_bound_to_another_lamp`, до первого кадра: две лампы на одном приборе
  запрещены и путём привязки.
- Worker переадресует заменитель на адрес отказавшего прибора и больше ничего на прибор
  не пишет. Реестр ставит на этот адрес **запись заменителя**: банки, DT8, группы и сцены
  в ней — то, что держит новый прибор. Из записи отказавшего, если выбран
  `metadata_and_overrides`, переходят имя, заметки и override'ы; остальное отбрасывается.
- Группы и сцены программирует обычный apply
  ([`groups.md`](groups.md) §`POST groups/apply`, [`scenes.md`](scenes.md)): строки
  лампы на этом адресе сравниваются уже с заменителем и становятся грязными, если он
  держит другое. Атрибуты 102 (fade, уровни) не переносятся.
- Лампа отказавшего прибора остаётся на адресе с тем же HA `unique_id`. Если у
  отказавшего лампы нет, лампа заменителя переходит вместе с ним. Если к моменту
  коммита привязаны обе, адрес остаётся за лампой отказавшего, а лампа заменителя
  отвязывается, как при `DELETE` прибора.
- **Старый прибор должен быть снят с шины заранее**: пока он отвечает на сохраняемом
  адресе, операция завершается `verify_failed`.
- Результат — `{failed_short_address, replacement_short_address,
  restored{metadata_and_overrides}}`.
- События: `PhysicalDeviceChangedEvent` обоих адресов и `VirtualLampChangedEvent` каждой
  лампы, чья привязка или прибор изменились; при лампе на адресе ещё
  `GroupMatrixChangedEvent` и `SceneMatrixChangedEvent` каждой сцены.

## `POST steps/{step}` — expert-шаги

Типизированные примитивы IEC для ручного коммиссионинга. Операции не создают.

| `step` | Тело | Добавки в ответе |
|---|---|---|
| `initialise` | `{scope?: all (по умолчанию) \| unaddressed \| short, short_address?}` | — |
| `randomise` | `{}` | — |
| `search-address` | `{search_address}` (24 бита) | — |
| `compare` | `{}` | `match` |
| `program-short-address` | `{short_address}` | — |
| `verify-short-address` | `{short_address}` | `match` |
| `query-short-address` | `{}` | `short_address`, `answer` |
| `withdraw`, `terminate` | `{}` | — |

- Неизвестный ключ тела — `400 unknown_field`; шаг, команду которого никто не принял,
  — `503 delivery_rejected`. Отказ до провода — как у любого синхронного маршрута:
  выключенный адаптер — `409 conflict` с `message = adapter_disabled`, пассивный
  контроллер — `409 controller_standby` с `Retry-After: 1`.
- Ответ — `success`, `backward_frame` и добавки шага; отсутствующее поле значит «к
  шагу не относится» или, при `success: false`, «шаг не исполнен». Отказ исполнения на
  проводе тоже приходит `200` с `success: false`, и тогда в ответе есть `error_code` и
  `message`, если исполнитель назвал причину (например, `preempted`).
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

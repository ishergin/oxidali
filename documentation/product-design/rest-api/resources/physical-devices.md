# Ресурс: Physical devices

Реальные DALI control gear по коротким адресам `0..63` адаптера: записи реестра,
метаданные и override'ы, чтение и запись атрибутов прибора, прямое управление и
discovery.

**Границы:** смена адреса, identify и замена прибора —
[`commissioning.md`](commissioning.md); пользовательская сущность поверх прибора —
[`virtual-lamps.md`](virtual-lamps.md); формы состояния —
[`../contracts/state-contracts.md`](../contracts/state-contracts.md); таксономия
атрибутов — [`../../bus-contracts/dali-attribute-taxonomy.md`](../../bus-contracts/dali-attribute-taxonomy.md).
BDD — [`physical_devices`](../../../../tests/dali2rust-bdd/features/physical_devices/).

У физического устройства нет сущности в Home Assistant: наружу его выставляет
привязанная виртуальная лампа.

## Маршруты

| Метод | Путь (`/api/v1/adapters/{adapter_id}/…`) | Назначение | Ответ |
|---|---|---|---|
| `GET` | `physical-devices` | Список (сводка) | `200` |
| `GET` | `physical-devices/{short}` | Ядро устройства | `200` |
| `GET` | `physical-devices/{short}/attributes[?sections=…]` | Секции атрибутов | `200` |
| `GET` | `physical-devices/{short}/memory-banks` | Покрытие прочитанных банков | `200` |
| `PATCH` | `physical-devices/{short}` | Метаданные, override'ы, разрешения | `200` |
| `DELETE` | `physical-devices/{short}` | Забыть запись | `204` |
| `POST` | `physical-devices/{short}/write-attributes` | Запись атрибутов в прибор | `202` `attribute_write` |
| `PUT` | `physical-devices/{short}/target-state` | Прямое управление по адресу | `200` |
| `POST` | `discovery-runs` | Discovery | `202` `discovery` |
| `POST` | `physical-devices/{short}/attribute-reads` | Чтение атрибутов и банков | `202` `attribute_read` |

`{short}` вне `0..63` — `400 invalid_resource_id`; записи нет — `404 not_found`.

## Поверхность чтения: четыре ресурса

Список, ядро, секции атрибутов и покрытие банков — разные ресурсы, потому что ответ
строится на стеке единственной задачи httpd: устройство целиком (двенадцать секций
атрибутов плюс банки) стоит килобайты и на весь список не помещается. Секции
сериализуются по одной.

- **Список** — строка на устройство: идентичность, effective тип и режим, capabilities,
  `state`, плюс вынутые плоско, без provenance, `groups_membership`, `gtin` и
  `identification_number` (пара — устойчивая идентичность прибора, её читают по всей
  шине одним запросом). `random_address` в строке остаётся: через него проверяют
  листинг discovery. `now_ms` — один на конверт.
- **Ядро** — идентичность, имя и заметки, тип и режим со всеми источниками и
  override'ами, `supported_device_types`, разрешения DT8, capabilities, `state` и
  `color_temperature_range`. Его же эхом отдают `PATCH` и `PUT …/target-state`.
- **Секции** — `?sections=a,b` выбирает из `common_102`, `dt6_led`, `dt8_color`,
  `extended`, `groups`, `scenes`, `memory_identity`, `memory_profile`,
  `memory_bus_unit`, `memory_luminaire`, `memory_energy`, `memory_diagnostics`; без
  параметра — все. Порядок ответа канонический (отфильтрованный ответ — срез
  полного). Неизвестное имя — `400 invalid_value`, а не молчаливый `{}`, который не
  отличить от непрочитанного прибора. Это имена **секций ответа**, а не план чтения
  `attribute_groups`.
- **Банки** — какие диапазоны каких банков успешно прочитаны и когда. Сырые байты
  банков наружу не отдаются.

Метки времени — в часах контроллера; возраст считается от `now_ms` ответа.

## Смысл полей ядра

- `device_type_*`, `color_mode_*`: `*_discovered` — evidence скана и чтения
  атрибутов, `*_override` — ручное значение (`null` — нет), `*_effective` — override
  или evidence, `*_source` — `manual_override` / `discovered`. Evidence **липкое** и
  **персистится**: сбойное чтение его не стирает, перезагрузка тоже.
- `supported_device_types` — **весь набор** типов прибора сырыми номерами DALI;
  классификация выше его не заменяет. **Нет поля** — перечисление ни разу не
  завершилось; **пустой список** — прибор сам ответил, что частей 2xx нет; когда тип
  попадает в набор и почему набор хранится —
  [09 §Device types](../../../architecture/09-dali-protocol-rules.md#device-types-and-memory-banks).
  Липкий и персистится.
- `capabilities` — подтверждённые железом биты плюс бит, который заявляет
  `color_mode_override` (см. ниже). `device_type_override` в capabilities не
  участвует.
- `dt8_auto_activation_repair`, `dt8_rgbwaf_control_assert` — поустройственные
  **исключения** из controller-global разрешений
  [`settings-dali.md`](settings-dali.md); всегда присутствуют, по умолчанию `true`.
- `color_temperature_range` — собственный диапазон Tc прибора в кельвинах; нет поля,
  пока его не прочитало чтение DT8 — клиент тогда берёт свой дефолт.

## Особые листья секций

- Обычный лист — `{value, source: readback | write_confirmed, last_read_ms?,
  last_write_confirmed_ms?}`; readback сохраняет метку подтверждённой записи и
  наоборот. `groups` — один лист `membership` (16-битная маска), `scenes` — листья
  `scene_0..scene_15`.
- `common_102.light_source_type` — сырой ответ `QUERY LIGHT SOURCE TYPE`, включая MASK;
  классифицирует его клиент. MASK значит «несколько источников», и тогда первые три
  приходят упакованными в `light_source_types` (`first << 16 | second << 8 | third`).
  Молчание DALI-1 прибора на этот запрос конформно и отсутствием прибора не считается.
- `memory_energy` / `memory_diagnostics` (DiiA Part 252/253) — листья с тремя
  состояниями: значение, «не реализовано» (MASK), «временно недоступно» (TMASK, с
  моментом начала — дольше 30 с это признак неисправности) и признаком насыщения
  счётчика. Три факта не схлопываются в один пустой. Частота питания `0` в
  `memory_diagnostics` — постоянный или выпрямленный ток (DiiA 253), а не «неизвестно».
- `memory_bus_unit` — банк 0 выше `0x1A`: байт конфигурации с классификацией и байт
  реализованных Part 15x — `{raw, parts}`, где `parts` — номера частей (бит x — Part 15x).
  Байт с битами вне диапазона части не заявляет: `parts: null`. Нет секции — прибор
  молчит на эти локации, что конформно.
- `memory_luminaire` — банк 1 выше `0x10` (DiiA Part 251), заполняется только при
  `content_format_id` 3, 4 или 5. Числовой лист — `{raw, value, part209_implemented}`:
  `value: null` — стандартное «неизвестно», отсутствие листа — «не читалось».
- Какие секции переживают перезагрузку —
  [`dali-attribute-taxonomy.md`](../../bus-contracts/dali-attribute-taxonomy.md).

## `PATCH`

Merge-patch. Пишутся:

- `name` (до 64 байт UTF-8) и `notes` (до 48 байт); `null` очищает. Превышение —
  `422 invalid_value` с числом байт в `message`; значение не строка и не `null` —
  `422 invalid_value`.
- `device_type_override` — `dt6_led` / `dt8_color` / `unknown` или `null`. **Не шире
  объявленного набора** (почему —
  [09 §Device types](../../../architecture/09-dali-protocol-rules.md#device-types-and-memory-banks)),
  иначе `422 invalid_value`; сужение (RGB-лента, которую ведут только яркостью) —
  назначение override'а. Пока набор не известен, проверки нет: иначе прибор без
  завершённого перечисления стал бы ненастраиваемым.
- `color_mode_override` — режим или `null`.
- `dt8_auto_activation_repair`, `dt8_rgbwaf_control_assert` — bool, не nullable.

Запрещены — `422 unsupported_field`: `state`, `attributes`, `memory_banks`,
`capabilities`, `short_address`, все `*_discovered` / `*_effective` / `*_source`,
`supported_device_types`. Атрибуты прибора (fade, power-on и т. п.) — тот же код с
подсказкой `write-attributes`. Адрес меняет только
[`commissioning.md`](commissioning.md).

Публикуется `PhysicalDeviceOverrideCommand` и, если есть `notes`, вторая команда
`PhysicalDeviceNotesUpdateCommand` — под одним дедлайном; атомарности между ними нет
(при частичной публикации — `503 partial_apply`, повтор безопасен). Изменение
effective-значений распространяется на привязанную лампу.

**`color_mode_override` — заявление о возможности.** Заявленный режим добавляет свой
бит к capabilities прибора и привязанной лампы: target-state, который без него
отбивался `unsupported_capability`, доходит до провода. Заявление не стирает
подтверждённые железом возможности и обратимо: сброс в `null` возвращает скан-evidence,
потому что бит добавляется при проекции, а не пишется в хранимое evidence.

## `DELETE` — забыть устройство

`204`, операции нет, на провод ничего не уходит: контроллер забывает свою запись, а не
перепрограммирует прибор. Удаление **не гейтится** по отсутствию прибора: провод не
отличает снятый светильник от обесточенного, поэтому защита — подтверждение в UI.

- Запись, атрибуты и банки удаляются.
- Привязка виртуальной лампы снимается; сама лампа (имя, HA-опция) остаётся. Без
  каскада реестр изменил бы форму на следующей загрузке: гидратация снимает привязку к
  отсутствующему устройству.
- Строки сцен и `desired` групп лампы не трогаются; applied-маска просто перестаёт
  находить устройство.
- Прибор, оставшийся на шине, вернётся следующим сканом **пустой** записью, без имени и
  привязки — поверхность обязана сказать это до подтверждения.

## `POST …/write-attributes`

Тело — любое подмножество записываемых атрибутов; отсутствующее поле не пишется:

| Поле | Диапазон | Прибор |
|---|---|---|
| `power_on_level`, `system_failure_level` | `0..254` | 102 |
| `fade_time_ms` | `0..90500`; округляется к коду Table 4, подтверждается длительность кода | 102 |
| `fade_rate` | `0..15` | 102 |
| `extended_fade_time_ms` | `0..65535` (пара множитель/база) | 102 |
| `min_level`, `max_level` | `0..254` | 102 |
| `tc_coolest_mirek`, `tc_warmest_mirek` | `1..65534` | DT8, rendered Tc-лимиты |
| `dimming_curve` | `0` логарифмическая, `1` линейная | DT6, `SELECT DIMMING CURVE` |

- `min_level > max_level` или `tc_coolest_mirek > tc_warmest_mirek` в одном теле —
  `422 invalid_value`: исполнитель пишет их по порядку, и пересечение границы на
  середине было бы каскадом, о котором никто не просил. Значение вне диапазона — `422
  invalid_value`; любое другое поле — `422 unsupported_field`.
- Каждая запись проверяется read-back'ом, и в реестр попадает **принятое** прибором, а
  не запрошенное (прибор вправе клампить). Поле, чей read-back ответил другим
  значением, остаётся без провенанса `write_confirmed`, и операция всё равно успешна: на
  проводе ничего не сломалось.
- Read-back без ответа не подтверждает ничего: поле сохраняет прежнее значение и
  провенанс, ответившие поля записываются, а операция завершается `failed` с
  `verify_unanswered` — запись могла и не дойти, и прочитать это можно только новым
  чтением атрибутов.
- Гейта по типу прибора у Tc-лимитов и кривой нет
  ([09 §Device types](../../../architecture/09-dali-protocol-rules.md#device-types-and-memory-banks)):
  Tc-лимиты прибора без DT8 остаются без подтверждения, кривая прибора без DT6 — тоже, с
  `verify_unanswered`. Контролы прячет UI.
- `dimming_curve` меняет смысл всех уже сохранённых в приборе уровней (сцены,
  power-on) — это конфигурационная запись, а не настройка контроллера.
- Override'ы типа и режима, членство в группах и сцены сюда не входят.

## `PUT …/target-state`

Прямое управление прибором по короткому адресу (`DaliSetTargetStateCommand` со
scope `Short`), синхронное подтверждение, ответ `200` с ядром устройства. Тело и
ошибки — как у виртуальной лампы
([`virtual-lamps.md`](virtual-lamps.md), [`../contracts/state-contracts.md`](../contracts/state-contracts.md));
цепочка — [`../workflows.md`](../workflows.md) §Target-state. Операции не создаёт.

## `POST /discovery-runs`

Тело `{"mode": …}` — `scan_known_short_addresses` · `refresh_known` ·
`commission_unaddressed` ([`../contracts/enums.md`](../contracts/enums.md)); диапазон
адресов клиент не задаёт. Неизвестный режим — `422 invalid_enum`.

- Прибор попадает в реестр только после подтверждения: стабильный random address,
  затем проверка пары «short + random» последовательностью поиска. При занятой шине
  проверка повторяется в пределах бюджета, так что спорное окно ответа не рождает
  фантом; подтверждённые раньше приборы сохраняются при позднем отказе.
- Прогон держит шину целиком и командам оператора не уступает
  ([`ADR-013`](../../../architecture/decisions/ADR-013-wire-priority-and-yield-granularity.md)).

## `POST …/attribute-reads`

Тело: `attribute_groups` — непустой список групп (план чтения,
[`../contracts/enums.md`](../contracts/enums.md)), и необязательный `memory_banks` —
пресет. Нет списка — `400 invalid_json`; пустой — `422 invalid_value`; неизвестное имя
группы или пресета — `422 invalid_enum`.

- Группы `groups` и `scenes` здесь — readback прибора в его атрибуты, а не
  adapter-level матрицы.
- Банки читаются **адресно**, не перебором: длина каждого — по собственному байту
  последнего адреса прибора, поэтому расширенный `identity` ничего не стоит прибору,
  который столько не объявляет. Если нет обязательного банка серии DiiA (202 для
  типа 51, 205 для типа 52), объявление типа неверно и остальные банки серии не
  читаются.
- Пресеты ([`../contracts/enums.md`](../contracts/enums.md) §Пресеты банков памяти)
  разделяют банки Part 252/253 по тому, как часто их имеет смысл читать: живую мощность
  (`power`) прибор обновляет не чаще раза в 30 с, а константам производителя
  (`luminaire_data`) хватает одного чтения на прибор.
- Чтение по запросу уступает командам оператора
  ([09 §Priority and yielding](../../../architecture/09-dali-protocol-rules.md#priority-and-yielding));
  итоги по группам — `attribute_read_outcomes` операции
  ([`../contracts/enums.md`](../contracts/enums.md)).

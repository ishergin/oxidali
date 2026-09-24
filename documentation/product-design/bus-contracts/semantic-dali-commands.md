# Семантические DALI-команды

Продуктовые модули публикуют намерение — «эта лампа на этот уровень», «эта ячейка
членства» — а `DaliWorker` превращает его в последовательность кадров IEC 62386.
Здесь — семейства команд и их смысл.

Границы: поля — объявления `declare_bus_payloads!` в `dali2rust-contracts/src/msg/`;
правила протокола на проводе (продуктовый и диагностический путь, приоритет, транзакции,
чтение ответов, адресация) —
[`../../architecture/09-dali-protocol-rules.md`](../../architecture/09-dali-protocol-rules.md);
очередь и гейты исполнителя —
[`../runtime-modules/dali-worker/README.md`](../runtime-modules/dali-worker/README.md);
кто какую команду публикует — [`semantic-dali-coverage-matrix.md`](semantic-dali-coverage-matrix.md).

## Правило продуктового пути

Продуктовый модуль публикует семантическую команду `Dali<Verb><Object>Command` /
`Dali103<Verb>Command` либо команду реестра, настроек или операции и никогда не
собирает опкод или кадр; сырой `DaliCommandPayload` — только диагностический путь
([09 §Product path](../../architecture/09-dali-protocol-rules.md#product-path-and-diagnostic-path)).
Новый продуктовый сценарий, отправляющий DALI, обновляет этот каталог, матрицу
покрытия, BDD-сценарий и контрактный тест.

Команды несут только бизнес-намерение; корреляция, происхождение и адаптер шины — в
конверте ([`commands.md`](commands.md) §Конверт). Цель внутри payload'а —
`DaliTargetScope` (`VirtualLamp`, `Short`, `Group`, `Broadcast`; `AddressRange`
зарезервирован) или `DaliProgramTarget` (`VirtualLamp` / `Short`) — это DALI-адресат,
а не маршрут шины. Уровень и цвет — в единицах продукта
([`shared-state-contracts.md`](shared-state-contracts.md)).

## Управление светом

**`DaliSetTargetStateCommand`** — единственная команда «поставить свет». Один
`LightSetpoint` (питание, уровень, цвет; смысл полей —
[`../rest-api/contracts/state-contracts.md`](../rest-api/contracts/state-contracts.md))
на лампу, короткий адрес, группу или broadcast.

- `VirtualLamp` разрешается в короткий адрес **воркером в момент исполнения**:
  перепривязка во время полёта уходит на новое устройство, а лампа, отвязанная к этому
  моменту, даёт `DaliTargetStateFailedEvent` с `vl_unbound`.
- Покомандного фейда нет: длительность любого перехода живёт в гире (`fadeTime`) и
  пишется через `DaliWriteAttributesCommand`.
- Подтверждение: для лампы и короткого адреса — после коммита реестра (цепочка
  applied-факт → проектор → RRUC → подтверждение реестра); для группы и broadcast —
  воркером сразу после исполнения, а проекция на членов идёт асинхронно.

**`DaliRecallLastActiveLevelCommand`** — `GO TO LAST ACTIVE LEVEL` на группу или
broadcast (продюсер — HCL для `level_mode = last_active`). Applied-факт несёт
«включено, уровень не указан»; уровень членов реестр предсказывает сам
([`../../architecture/06-registry-and-persistence.md`](../../architecture/06-registry-and-persistence.md)).
Несогласованные scope и группа отвергаются (`409`) до провода.

**`DaliStopFadeCommand`** — один кадр `DAPC(MASK)` на любой класс адреса
(102 §9.5.9): таймер фейда стоит, `targetLevel` становится `actualLevel`. MASK — не
уровень, поэтому это отдельный вид, а не форма target-state; проецировать нечего,
реестр сохраняет прежнее значение до следующего чтения.

**`DaliRecallSceneCommand`** — единственная продуктовая активация сцены: нативный
`GO TO SCENE`, яркость и цвет одним кадром. Scope — `Broadcast` или `Group`;
одиночная лампа отвергается (`invalid_recall_scope`) продуктовым решением —
поламповое управление это target-state. Recall никогда не заменяется синтетическим
поламповым target-state или программированием сцены. Broadcast-recall ставит активную
сцену адаптера, групповой — снимает её.

## Программирование конфигурации

**`DaliProgramGroupMembershipCommand`** — одна ячейка «устройство × группа»
(`Add` / `Remove`). Публикует только apply orchestrator, разворачивая diff матрицы
(`ADR-007`). После записи воркер читает `QUERY GROUPS` и публикует маску; реестр
выводит из неё `applied`. Непривязанная лампа не программируется: ячейка
пропускается и остаётся dirty.

**`DaliProgramSceneCommand`** — одна строка сцены (`Write`, `Update`, `Clear`).
Целевое состояние несёт `ColorValue` целиком, со всеми шестью каналами RGBWAF, и без
провенанса — это запрограммированное, а не наблюдённое состояние. После записи
воркер читает уровень сцены и, для цветного гира, её цвет через регистры REPORT.
`error = None` ⟺ строка запрограммирована; запрограммированная строка без
подтверждённого readback'а оставляет derived `applied` прежним.

**`DaliWriteAttributesCommand`** — скалярные записываемые атрибуты устройства (перечень,
диапазоны и что попадает в реестр —
[`physical-devices.md`](../rest-api/resources/physical-devices.md) §write-attributes),
каждый — отдельной записью с readback'ом; `DaliAttributesWrittenEvent` несёт
подтверждённые значения. Членство в группах и сцены сюда не входят — у них свои
семейства.

## Чтение и обнаружение

- **`DaliDiscoverDevicesCommand`** — режимы `ScanKnownShortAddresses`,
  `CommissionUnaddressed`, `RefreshKnown` на один адаптер.
- **`DaliReadAttributesCommand`** — один короткий адрес: маска групп атрибутов плюс
  пресет банков памяти. Результат — чанк на группу и итог исходов по группам
  ([`dali-attribute-taxonomy.md`](dali-attribute-taxonomy.md)).
- **`DaliReadMemoryBankCommand`** — внутренний путь; публичный доступ к банкам — через
  пресет чтения атрибутов.
- **`DaliBusHealthProbeCommand`** — широковещательный опрос сегмента об отказах ламп
  с положительным контролем (правило чтения —
  [09 §Reading answers](../../architecture/09-dali-protocol-rules.md#reading-answers)).
  Публикует поллер.

## Коммиссионинг control gear

Маршруты, тела, взаимное исключение на адаптере и результаты —
[`../rest-api/resources/commissioning.md`](../rest-api/resources/commissioning.md).

- **`DaliIdentifyDeviceCommand`** — процедура опознания самого гира
  ([09 §Faults and identification](../../architecture/09-dali-protocol-rules.md#faults-and-identification));
  механизм фиксируется в результате операции.
- **`DaliAddressingCommand`** — смена короткого адреса без `INITIALISE`
  ([09 §Addressing control gear](../../architecture/09-dali-protocol-rules.md#addressing-control-gear)).
- **`DaliReplaceDeviceCommand`** — перенос адреса отказавшего прибора на заменитель с
  восстановлением выбранных слайсов; флаги `restored_*` сообщают фактически
  восстановленное.
- **`DaliCommissioningStepCommand`** — экспертный примитив IEC; ответ request-scoped,
  операции нет.

## Part 103 — устройства ввода

Кадры, адресация и кодировки —
[09 §Part 103](../../architecture/09-dali-protocol-rules.md#part-103-control-devices);
исполнители — в `DaliWorker` (`dev103`, `ADR-016`); модуль —
[`../runtime-modules/input-devices/README.md`](../runtime-modules/input-devices/README.md).

| Команда | Что делает | Прерываемость |
|---|---|---|
| `Dali103ScanCommand` | Перечисляет адресованные устройства и их инстансы | по кадру |
| `Dali103CommissionCommand` | Адресует устройства в сессии `INITIALISE` | никогда |
| `Dali103InstanceConfigureCommand` | Схема событий, фильтр, группы, приоритет, таймеры, `instanceActive` — каждое поле отдельной записью с readback'ом; первый отказ проваливает операцию, доказанные поля публикуются | по шагу |
| `Dali103IdentifyCommand` | Процедура опознания самого устройства | по кадру |
| `Dali103FeedbackConfigureCommand` | NVM-переменные видимой индикации Part 332; каждое поле с readback'ом | по шагу |
| `Dali103FeedbackDriveCommand` | `ACTIVATE` / `STOP` / `SELECT FEEDBACK` одним кадром; REST-двери нет | по кадру |
| `Dali103ArbitrationProbeCommand` | Широковещательный `QUERY APPLICATION CONTROLLER ENABLED` (DiiA 351 §7) на приоритете 5; единственный вид, проходящий гейт пассивного контроллера | никогда |
| `Dali103HandoverCommand` | Пара `ENABLE APPLICATION CONTROLLER` пиру, затем собственный уход в пассив; направление одно (103 §9.9.1) | никогда |

Какие исходы 24-битного кадра повторяются —
[`ADR-027`](../../architecture/decisions/ADR-027-dtr-operand-proof-and-readback-outcomes.md).

## Приоритет и прерываемость

Приоритет провода, гранулярность прерывания и класс IEC 62386-103 §9.13.1 берутся из
**вида** команды (таблица `WIRE_CLASSES`), а происхождение уточняет несколько видов —
[09 §Priority and yielding](../../architecture/09-dali-protocol-rules.md#priority-and-yielding).

## Команда → результат

| Команда | Результат | Дальше |
|---|---|---|
| `DaliSetTargetStateCommand` | `DaliTargetStateAppliedEvent` / `DaliTargetStateFailedEvent` | проектор → RRUC → `RuntimeStateChangedEvent` |
| `DaliRecallLastActiveLevelCommand` | applied-факт без уровня | проектор → RRUC; уровень из тени реестра |
| `DaliStopFadeCommand` | подтверждение | ничего не проецируется |
| `DaliRecallSceneCommand` | `DaliSceneRecalledEvent` | проектор раскрывает по applied-строкам сцены |
| `DaliProgramGroupMembershipCommand` | `DaliGroupMembershipProgrammedEvent` | реестр: applied-членство, `GroupMatrixChangedEvent` |
| `DaliProgramSceneCommand` | `DaliSceneProgrammedEvent` (+ чанки цвета сцены) | реестр: applied-строка, `SceneMatrixChangedEvent` |
| `DaliWriteAttributesCommand` | `DaliAttributesWrittenEvent` | прямой коммит реестра, `PhysicalDeviceChangedEvent` |
| `DaliDiscoverDevicesCommand` | прогресс скана, итог сверки, сигнал операции | реестр: записи устройств |
| `DaliReadAttributesCommand` | чанки атрибутов, чанки банков, исходы по группам | реестр (доказательства) и проектор (runtime) |
| `DaliBusHealthProbeCommand` | `DaliBusHealthProbedEvent` | счётчики поллера, дисплей |
| коммиссионинг | `DaliDeviceIdentifiedEvent`, `DaliAddressingCompletedEvent`, `DaliDeviceReplacedEvent`; шаг — подтверждение | трекер; реестр переносит запись |
| Part 103 | прогресс скана, `Dali103InstanceConfiguredEvent`, сигнал операции | реестр устройств ввода |
| арбитраж | `Dali103ArbitrationProbedEvent`, `Dali103HandoverSentEvent` | воркер арбитража |

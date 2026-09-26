# События шины

Семейства событий: кто публикует, кто потребляет, с какой гарантией доставки и что
потребитель вправе предполагать. События описывают факты, которые уже произошли.

Границы: форма payload'ов — объявления `declare_bus_payloads!`
(`dali2rust-contracts/src/msg/events.rs`); команды — [`commands.md`](commands.md);
семантика DALI-результатов со стороны исполнителя —
[`semantic-dali-commands.md`](semantic-dali-commands.md); механика маршрутизации —
[`../../architecture/03-bus-and-backpressure.md`](../../architecture/03-bus-and-backpressure.md).

## Доставка

События маршрутизируются по объявленному виду, а событие — единственный носитель факта —
публикуется через `publish_required`; механика обоих правил —
[`../../architecture/03-bus-and-backpressure.md`](../../architecture/03-bus-and-backpressure.md).
Класс каждого вида — столбец «Доставка» ниже: `required` — факт, который перечитать
неоткуда; best-effort — уведомление, после потери которого потребитель перечитывает
состояние.

**Имена**: `Dali<Object><Verb>Event` для результатов DALI, `<Resource>ChangedEvent` для
реестра; суффикс `Event` обязателен.

## Семейства

| Семейство | События | Издатель | Потребители | Доставка |
|---|---|---|---|---|
| Изменения реестра | `AdapterSettingsChangedEvent`, `VirtualLampChangedEvent`, `PhysicalDeviceChangedEvent`, `GroupChangedEvent`, `GroupMatrixChangedEvent`, `SceneChangedEvent`, `SceneMatrixChangedEvent`, `InputDeviceChangedEvent` | registry worker | WS; MQTT (лампы, устройства, группы, матрица групп, сцены, устройства ввода); правила (лампы, устройства) | best-effort |
| Расписания HCL | `HclScheduleChangedEvent` | registry worker | HCL, дисплей | best-effort |
| Runtime | `RuntimeStateChangedEvent` | registry worker | WS, MQTT, HCL, правила | best-effort |
| Настройки | `PollerSettingsChangedEvent`, `DaliSettingsChangedEvent`, `HomeAssistantSettingsChangedEvent`, `RedundancySettingsChangedEvent`, `PoliciesChangedEvent` | registry worker | поллер; арбитраж, супервизор и правила; — ; воркер репликации; — | best-effort |
| Операции | `OperationStatusChangedEvent` | operation tracker | WS, дисплей | best-effort |
| Сигналы исполнителей | `OperationWorkerSignalEvent` | `DaliWorker`, реестр, оркестратор, MQTT, правила, OTA, HTTP | operation tracker | **required** |
| Результаты DALI | [ниже](#результаты-dali) | `DaliWorker` | по виду | по виду |
| Наблюдения | `DaliObservedFrameEvent` | sniffer translator | проектор, реестр, дисплей | best-effort |
| Вход Part 103 | `DaliInputEventObservedEvent`, `DaliInputDeviceLifecycleEvent`, `Dali103ApplicationControlObservedEvent` | sniffer translator | реестр; правила, MQTT, WS, дисплей (по виду) | **required** |
| Результаты Part 103 | `Dali103ScanStartedEvent`, `Dali103ScanProgressEvent`, `Dali103InstanceConfiguredEvent` | `DaliWorker` | реестр; правила (`manual config changed`) | **required** |
| Отказоустойчивость | [ниже](#отказоустойчивость) | `DaliWorker`, арбитраж, реестр | по виду | по виду |
| Правила | `RulesChangedEvent`, `RulesActivationEvent` | rules worker | WS | best-effort |
| Home Assistant | `HomeAssistantDiscoveryPublishedEvent` | MQTT bridge | operation tracker | **required** |
| Загрузка и сервис | `IpAddressAssignedEvent`, `PersistenceLoadResultEvent`, `StatsReportedEvent`, `DaliEventPayload` | композиция; `DaliWorker` | дисплей; остальные observed-only | best-effort |

## Изменения реестра

- **Уведомление, а не снимок.** Payload изменённого ресурса — только идентичность
  (`adapter_id` + id); потребитель перечитывает состояние через read-port. Поэтому
  потерянное событие стоит устаревшего экрана до следующего изменения, а не
  испорченного состояния.
- Одно событие на эффективный коммит; чанковая серия — одно на коммит серии.
- `GroupMatrixChangedEvent` не называет группы, которые сдвинул: потребитель,
  которому это важно (мост Home Assistant), проходит по всем группам адаптера.

## `RuntimeStateChangedEvent`

Единственное событие о volatile-состоянии лампы; публикуется реестром **только после**
коммита `RegistryRuntimeUpdateCommand`. MQTT, WebSocket, HCL и правила строят видимое
состояние из него, а не из результатов DALI и не из наблюдений сниффера.

- `state_setpoint` / `state_observation` — **снимок записи после коммита**: `None`
  здесь значит «никому не известно».
- `commit_source` и `commit_dimensions` описывают **сам коммит**: кто его совершил и
  какие размерности (яркость, цвет) он заявил. Судить по снимку о том, что сделал
  коммит, нельзя: гир, несущий цвет от HCL, ответил бы «коммит заявил цвет» на любой
  чужой `DAPC`. На этих двух полях стоит override-логика HCL.
- `virtual_lamp_id` заполняется и для коммита по short address: реестр делает
  обратный поиск привязки, потому что потребитель разрешить её сам не может. Для
  устройства без привязки — `None`.
- `correlation_id` в payload'е нет — он в конверте.

## Операции

- `OperationStatusChangedEvent` — каждый переход статуса операции с ключом и
  компактной ошибкой ([`../runtime-modules/operation-tracker/README.md`](../runtime-modules/operation-tracker/README.md)).
- `OperationWorkerSignalEvent` (`WorkerStarted` / `WorkerSucceeded` / `WorkerFailed`
  по workflow correlation id) — единственный путь, которым операция заканчивается.

## Результаты DALI

| Событие | Потребители | Доставка | Что значит |
|---|---|---|---|
| `DaliTargetStateAppliedEvent` | проектор, реестр, дисплей | required | Уставка стала истинной на проводе; несёт scope, идентичности, setpoint, признак DAPC и монотонную метку момента применения |
| `DaliTargetStateFailedEvent` | дисплей | best-effort | Отказ исполнения; ждущий узнаёт о нём по подтверждению |
| `DaliSceneRecalledEvent` | проектор, реестр, правила, дисплей | required | На шине произошёл нативный recall; реестр запоминает активную сцену |
| `DaliGroupMembershipProgrammedEvent` | реестр, трекер, оркестратор | required | Ячейка членства запрограммирована; несёт маску readback'а |
| `DaliSceneProgrammedEvent` | реестр, трекер, оркестратор | required | Строка сцены запрограммирована; несёт readback уровня и эхо записанного |
| `DaliAttributesReadEvent` | реестр, проектор | required | Один чанк на группу атрибутов; секцию `RuntimeStatus` применяет только проектор |
| `DaliAttributeReadOutcomesEvent` | трекер, проектор, поллер | required | Исход каждой группы одного чтения; публикуется и на успехе, и на отказе, до терминального сигнала |
| `DaliAttributesWrittenEvent` | реестр, оркестратор | required | Принятое гиром (readback), а не запрошенное; на каждую исполненную запись, с ошибкой исполнения, если она была |
| `DaliMemoryBankReadEvent` / `...AbortedEvent` | реестр | required | Чанки одного логического чтения банка / отмена staging'а |
| `DaliDiscoveryProgressEvent` | реестр, дисплей | required | Одно устройство скана, по мере обхода |
| `DaliDiscoveryScanReconciledEvent` | реестр | required | Итог чистого скана известных адресов: маска подтверждённых |
| `DaliDiscoveryCompletedEvent` / `...FailedEvent` | — | best-effort | Observed-only: исход операции несут сигналы |
| `DaliDeviceIdentifiedEvent`, `DaliAddressingCompletedEvent`, `DaliDeviceReplacedEvent` | трекер; реестр (адресация, замена) | required | Терминальные результаты коммиссионинга, провал — тем же событием |
| `DaliBusHealthProbedEvent` | поллер, дисплей | best-effort | Итог широковещательного health-probe: контроль ответил и вердикт `Clear` / `One` / `Several` |

Правила, которые потребитель вправе предполагать:

- **Runtime — только через проектор** и `RegistryRuntimeUpdateCommand`
  ([`../runtime-modules/state-fanout/README.md`](../runtime-modules/state-fanout/README.md));
  доказательства (чанки чтения, запись, итог скана, чтение банка) реестр коммитит сам
  и публикует `PhysicalDeviceChangedEvent`.
- **Чанки банка памяти**: одно логическое чтение порождает несколько событий, каждое
  несёт начальное смещение всего чтения; реестр копит их по correlation id и коммитит
  только на чанке с `last_chunk`. Провал или таймаут отменяет staging, состояние
  операции несёт `OperationStatusChangedEvent`.
- **Набор типов устройства** едет в прогрессе скана как `Option<DeviceTypeSet>`; что
  значат `None` и пустой набор и как реестр их сливает —
  [`../../architecture/09-dali-protocol-rules.md`](../../architecture/09-dali-protocol-rules.md)
  §Device types and memory banks.
- **Сцена**: как из readback'ов программирования выводится applied-строка —
  [`../rest-api/contracts/state-contracts.md`](../rest-api/contracts/state-contracts.md)
  §`SceneRow`. Recall не меняет applied-матрицу.
- **Метки времени** applied-факта, recall, наблюдения и секции `RuntimeStatus`
  упорядочивают наблюдения в реестре
  ([`../../architecture/06-registry-and-persistence.md`](../../architecture/06-registry-and-persistence.md)
  §Merge rules).

## Наблюдения сниффера

`DaliObservedFrameEvent` публикует только sniffer translator; какие кадры он понимает —
[`../runtime-modules/sniffer-translator/README.md`](../runtime-modules/sniffer-translator/README.md).

- Публикуются только понятые кадры: `TargetStateObserved`, `SceneRecallObserved`,
  `LevelTransitionObserved`, `SceneWriteObserved` и `SceneRemovalObserved`. Вариант
  `UnknownObserved` в контракте остаётся, у проектора есть счётчик на случай нового
  производителя.
- `raw_frame` и `decode_status` — диагностическое свидетельство, не продуктовые
  данные.
- Раскрывает наблюдение проектор
  ([`../runtime-modules/state-fanout/README.md`](../runtime-modules/state-fanout/README.md));
  реестр по групповым кадрам взводит «группой командовали» для плиток Home Assistant.
  Запись сцены свет не меняет: её проектор пропускает, а реестр сбрасывает уровень
  сцены прибора ([`../runtime-modules/registry/README.md`](../runtime-modules/registry/README.md)).
  MQTT и WebSocket не обновляют видимое состояние из наблюдения напрямую.

## Вход Part 103

Нажатие, переход датчика или отчёт о питании панели перечитать неоткуда, поэтому
все три события транслятор публикует через `publish_required`.

- `DaliInputEventObservedEvent` — декодированное событие инстанса: схема, адресная
  информация источника, тип, сырые 10 бит и типизированное значение. Тип инстанса для
  схемы 2 подставляет транслятор — ниже по течению его уже не восстановить
  ([`../runtime-modules/sniffer-translator/README.md`](../runtime-modules/sniffer-translator/README.md));
  схема 0 идентичности устройства не несёт
  ([`../../architecture/09-dali-protocol-rules.md`](../../architecture/09-dali-protocol-rules.md)).
- `DaliInputDeviceLifecycleEvent` — служебные события устройства (power cycle,
  локальная перенастройка Part 333).
- `Dali103ApplicationControlObservedEvent` — подтверждённая send-twice пара чужого
  `ENABLE` / `DISABLE APPLICATION CONTROLLER` и её `DeviceCommandScope`; реестр решает,
  касается ли она нас: `Unaddressed` — только пока у контроллера нет короткого адреса.

Результаты команд Part 103 (`Dali103Scan*`, `Dali103InstanceConfiguredEvent`)
реестр проецирует в записи устройств ввода; флаг Part 333 из
`Dali103InstanceConfiguredEvent` — вход триггера правил `manual config changed`.

## Отказоустойчивость

- `Dali103ArbitrationProbedEvent` (best-effort, воркер арбитража) — вердикт зонда
  ([`../runtime-modules/redundancy/README.md`](../runtime-modules/redundancy/README.md)
  §Обнаружение).
- `Dali103HandoverSentEvent` (best-effort, воркер арбитража) — пара `ENABLE` пиру
  отправлена, можно уходить в пассив.
- `RedundancyTransitionEvent` (best-effort, правила: `controller becomes active`) —
  смена роли с причиной и метками; журнал переходов хранит сам воркер арбитража и
  отдаёт REST.
- `RegistrySliceReloadedEvent` (required, правила) — слайс-стор на узле изменился и
  реестр перечитал себя. Это событие, а не команда, потому что команда доходит ровно
  до одного владельца, а перечитать своё обязан каждый держатель персистентного
  состояния. Имя слайса в payload'е — повод, а не фильтр: перечитывать нужно по
  самому факту события.

Роль контроллера событием не передаётся — её несут заголовок `X-Dali2rust-Role` и
`/api/v1/health`.

## Настройки

- `PollerSettingsChangedEvent` и `DaliSettingsChangedEvent` — **снимки** применённых
  значений. Поллер перенастраивается по своему событию без рестарта; `DaliWorker` читает
  настройки DALI из read-port в момент применения, а событие будит арбитраж,
  супервизор и правила.
- `HomeAssistantSettingsChangedEvent` — уведомление без секрета (`enabled` и два
  флага цены: нужен ли реконнект, нужен ли переанонс). Мост на него сознательно не
  подписан (`ADR-015`).
- `RedundancySettingsChangedEvent` будит простаивающий воркер репликации — свежий
  `peer_url` пробуется сразу. Арбитраж и применение политик читают свои настройки
  из read-port и событием не управляются.

## Загрузка и сервис

- `IpAddressAssignedEvent` — адрес получен; потребитель — дисплей.
- `PersistenceLoadResultEvent` — по одному на слайс при гидрации на старте
  (`Loaded` / `Defaults` / `Failed`, при `Failed` применены умолчания). Сводка
  гидрации на шину не публикуется.
- `StatsReportedEvent` — объявлен, издателя нет
  ([`../runtime-modules/stats/README.md`](../runtime-modules/stats/README.md)).
- `DaliEventPayload` — сырое эхо диагностического пути.

Подключения WebSocket и MQTT событий не порождают — это счётчики `/api/v1/diagnostics`.

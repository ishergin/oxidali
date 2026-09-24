# Команды шины

Семейства команд, их владельцы и семантика, которую не видно из типов: кто ждёт
ответа, как длинные записи режутся на кадры, что потребитель вправе предполагать.

Границы: форма каждого payload'а — объявление в `declare_bus_payloads!`
(`dali2rust-contracts/src/msg/commands.rs`), и этот документ поля не перечисляет.
Семантические DALI-команды и Part 103 — [`semantic-dali-commands.md`](semantic-dali-commands.md);
события — [`events.md`](events.md); механика каналов, ёмкостей и маршрутизации —
[`../../architecture/03-bus-and-backpressure.md`](../../architecture/03-bus-and-backpressure.md).

## Как команда доходит до исполнителя

Один владелец на вид, синтетический `DeliveryRejected` вместо молчаливой потери и кадр
не больше 128 байт `postcard` — механика в
[`../../architecture/03-bus-and-backpressure.md`](../../architecture/03-bus-and-backpressure.md)
и [`../../architecture/04-contracts-and-api-bridge.md`](../../architecture/04-contracts-and-api-bridge.md),
рецепт нового вида — [`../../architecture/11-extension-recipes.md`](../../architecture/11-extension-recipes.md).
Всё, что больше кадра, едет серией со staging'ом и закрывающей скобкой (ниже).

## Дисциплины ответа

Как маршрут отвечает, зависит от вида команды, а не от того, кто её прислал. REST-сторона
тех же дисциплин — [`../rest-api/stability-and-versioning.md`](../rest-api/stability-and-versioning.md)
§Дисциплины ответа; кто слышит отказ доставки —
[`../../architecture/03-bus-and-backpressure.md`](../../architecture/03-bus-and-backpressure.md)
§A refusal needs a listener.

| Дисциплина | Кто ждёт | Чем заканчивается |
|---|---|---|
| Подтверждение запроса | HTTP-хендлер, один дедлайн на запрос | `DeliveryStatus` на канале подтверждений |
| Подтверждение + apply-watch | Хендлер настроек | Подтверждение, затем счётчик применения реестра |
| `202` + операция | Никто; оператор смотрит операцию | Только `OperationWorkerSignalEvent` |
| Внутренний факт | Никто | `CORRELATION_NONE`; реестр не подтверждает |

## Семейства

| Семейство | Виды | Владелец | Кто публикует | Дисциплина |
|---|---|---|---|---|
| Конфигурация реестра | адаптер; виртуальная лампа (метаданные, bind/rebind/unbind, удаление записи); физическое устройство (override, notes, удаление); устройство ввода (метаданные, notes); метаданные группы и сцены | registry worker | HTTP | подтверждение + apply-watch |
| Настройки контроллера | поллер, DALI, Home Assistant (настройки, пароль брокера, префиксы топиков, `controller_id`), отказоустойчивость, политики | registry worker | HTTP | подтверждение + apply-watch |
| Чанковые записи | матрица групп и матрица сцены (patch/replace) + `ConfigWriteCommitCommand`; `HclScheduleUpsertCommand` / `HclScheduleDeleteCommand` | registry worker | HTTP | `202` + операция `config_write` |
| Runtime реестра | `RegistryRuntimeUpdateCommand`, `RegistryLevelTransitionCommand` | registry worker | только проектор state-fanout | подтверждение (short/VL) или внутренний факт |
| Перечитать слайс | `RegistrySliceReloadCommand` | registry worker | импорт конфигурации (HTTP), воркер репликации | подтверждение |
| Семантические DALI (102/207/209) | `Dali*Command` для control gear | `DaliWorker` | HTTP, MQTT, HCL, поллер, правила, оркестратор | по виду, см. [`semantic-dali-commands.md`](semantic-dali-commands.md) |
| Part 103 | `Dali103*Command` | `DaliWorker` | HTTP, правила, арбитраж | по виду, там же |
| Диагностический сырой кадр | `DaliCommandPayload` (`raw_mode`) | `DaliWorker` | только диагностический REST | подтверждение запроса |
| Операции | `OperationBeginCommand`, `OperationRegistryResetCommand` | operation tracker | HTTP, оркестратор | — |
| Массовое применение | `GroupApplyExecuteCommand`, `SceneApplyExecuteCommand`, `PolicyApplyExecuteCommand` | apply orchestrator | HTTP, правила (`scene.apply`) | `202` + операция |
| HCL | `HclOverrideClearCommand` | HCL scheduler | HTTP, правила (`hcl.resume`) | подтверждение запроса |
| Home Assistant | `HomeAssistantDiscoveryPublishCommand`, `MqttPublishCommand` | MQTT bridge | HTTP; правила (`mqtt.publish`) | `202` + операция; без ответа |
| Правила | `RuleStageCommand`, `RuleCommitCommand`, `RuleEnableCommand`, `RuleRunCommand` | rules worker | HTTP | `202` + операция `config_write`; включение — ожидание ревизии |
| Прошивка | `FirmwareUpdateBeginCommand` | OTA worker | HTTP | `202` + операция `firmware_update` |

`OperationRegistryResetCommand` (терминирует все активные операции с
`registry_reset`) продового издателя не имеет.

## Конфигурационные команды

- **Маска patch.** Команды настроек и метаданных несут `patch_mask`; биты — связанные
  константы `PATCH_*` на типе. Бит поля, снятого с контракта, остаётся
  зарезервированным и не переиспользуется. Маска `PollerSettingsUpdateCommand`
  занята целиком: следующий флаг потребует расширить поле.
- **Текст — в байтах.** Имена и заметки — `FixedText*` фиксированной ёмкости;
  переполнение отвергается на REST-границе `422 invalid_value`, и отказ называет
  обе величины в байтах (кириллица занимает два байта на символ).
- **Заметки — отдельная команда.** `notes` физического устройства и устройства ввода
  едут своими командами, потому что оба текста в одном payload'е не помещаются в
  бюджет кадра. PATCH, несущий и `notes`, и другие поля, публикует две команды под
  одним дедлайном и атомарным не является.
- **Пароль брокера** ходит только в `HomeAssistantCredentialsUpdateCommand` — это
  единственный payload шины, несущий секрет; событие настроек его не несёт.
- `HomeAssistantControllerIdUpdateCommand` маски не имеет — публикация и есть patch;
  пустой `controller_id` отвергается на REST. Что мост делает при смене префиксов и
  `controller_id` — [`../runtime-modules/mqtt-home-assistant/README.md`](../runtime-modules/mqtt-home-assistant/README.md).
- **Удаление.** `VirtualLampDeleteCommand` удаляет запись лампы, а не привязку;
  `PhysicalDeviceDeleteCommand` забывает устройство и каскадом снимает привязки ламп.
- **Политики** (`PoliciesUpdateCommand`): `UNMANAGED` — это отсутствие уровня, а не
  уровень; применение ко всем устройствам — отдельная `PolicyApplyExecuteCommand`,
  которую разворачивает оркестратор.

## Чанковые записи

Матрица групп (64 × 16), матрица сцены и расписание HCL не помещаются в кадр. Решение и
его причины — [`ADR-012`](../../architecture/decisions/ADR-012-async-chunked-config-writes.md).

**Матрицы.** `GroupMatrixDesired{Patch,Replace}Command` и
`SceneMatrixDesired{Patch,Replace}Command` только складывают строки в staging реестра:
ни применения, ни бампа ревизии, ни события. Серию закрывает
`ConfigWriteCommitCommand`, и коммит происходит целиком или не происходит:

- все кадры серии несут один correlation id — id закрывающей команды; под ним же HTTP
  публикует `OperationBeginCommand`, и реестр закрывает операцию прямо из коммита;
- закрывающая команда несёт число чанков, и реестр сверяет его со staged-счётчиком;
- staging ключуется ресурсом и помнит свою серию: чанк другой серии выбрасывает
  незакрытый staging, а скобка закрывает только свою серию — из двух одновременных
  записей выигрывает последняя, скобка старой отвергается;
- брошенная серия выселяется по возрасту.

**Расписание HCL** — собственный протокол без закрывающей команды: заголовок
расписания повторяется в каждом чанке; чанк с нулевыми начальными индексами targets и
points открывает серию; индексы остальных обязаны равняться накопленной длине
(`409 chunk_out_of_order`); чанк с `last_chunk` коммитит, и
`HclScheduleChangedEvent` публикуется один раз — на коммите или удалении. Координаты
едут микроградусами (`i32`), чтобы payload оставался `Eq`. Реестр как единственный
писатель отвергает неисполнимое расписание (`422`) и девятое расписание
(`409 schedule_limit_reached`). Staging не персистится и выселяется по возрасту.

**Документ правил** — та же схема staging + скобка: `RuleStageCommand` несёт куски
исходника, `RuleCommitCommand` применяет документ одним разбором, одной заменой
набора, одним бампом ревизии и одним `RulesChangedEvent`. Новая серия (другой
correlation id) вытесняет незакрытую. Серия может быть длиннее ingress-очереди,
поэтому HTTP публикует её с ограниченным ретраем (`publish_required`).

## Runtime-команды реестра

`RegistryRuntimeUpdateCommand` (RRUC) — единственный вход, меняющий volatile
runtime-состояние; единственный продовый издатель — проектор state-fanout. Как реестр
сливает запись (частичное наблюдение, порядок по `observed_at_mono_ms`, отсутствие,
цвет) и какие факты несут корреляцию запроса —
[`../../architecture/06-registry-and-persistence.md`](../../architecture/06-registry-and-persistence.md).

**Идентичность** записи — только `virtual_lamp_id`, только `short_address` или оба,
когда один факт относится к привязанной лампе и её устройству и коммитится одной
записью. Запись только с `virtual_lamp_id` для непривязанной лампы отвергается
(`vl_unbound`).

`RegistryLevelTransitionCommand` несёт глагол, а не уровень: чужую арк-команду,
которая двигает уровень, не называя его (`RECALL MAX/MIN LEVEL`, четыре шаговые,
`GO TO LAST ACTIVE LEVEL`; IEC 62386-102 §11.3). Шаговые глаголы определены через
`targetLevel`, поэтому разрешить их может только единственный писатель реестра —
чтением-модификацией-записью своей копии; производитель, посчитавший уровень сам,
терял бы шаги при потоке кадров от поворотной ручки. `UP`/`DOWN` не проецируются:
их результат стандарт не определяет.

## Команды и операции

Виды, чья дисциплина — `202` + операция, и тип операции, который видит оператор:

| Тип операции | Команды |
|---|---|
| `discovery` | `DaliDiscoverDevicesCommand`, `Dali103ScanCommand` |
| `attribute_read` | `DaliReadAttributesCommand` (банки памяти — через пресет в этой же команде) |
| `attribute_write` | `DaliWriteAttributesCommand` с `signals_operation = true` |
| `group_apply` / `scene_apply` | `GroupApplyExecuteCommand` / `SceneApplyExecuteCommand` → ячейки `DaliProgram*Command` |
| `policy_apply` | `PolicyApplyExecuteCommand` → ячейки `DaliWriteAttributesCommand` с `signals_operation = false` |
| `commissioning_identify` | `DaliIdentifyDeviceCommand`, `Dali103IdentifyCommand` |
| `commissioning_address_change` | `DaliAddressingCommand`, `Dali103CommissionCommand` |
| `commissioning_replace_device` | `DaliReplaceDeviceCommand` |
| `config_write` | чанковые записи матриц и HCL, документ и ручной запуск правил, конфигурация инстансов и индикации Part 103 |
| `ha_discovery_publish` | `HomeAssistantDiscoveryPublishCommand` |
| `firmware_update` | `FirmwareUpdateBeginCommand` |

`signals_operation` говорит, чья операция: у маршрута `write-attributes` терминал —
сигнал `DaliWorker`, у ячейки применения политики корреляция принадлежит прогону, и
операцию закрывает оркестратор после последней ячейки. Тип `memory_bank_read`
объявлен, но публичного маршрута у `DaliReadMemoryBankCommand` нет.

## Конверт

`BusEnvelope` несёт транспорт и маршрутизацию; payload'ы эти поля не дублируют.

- `correlation_id` — связь с ожидающим подтверждением или с операцией;
  `CORRELATION_NONE` — факт, которого никто не ждёт.
- `origin` — кто попросил: `Api`, `Mqtt`, `Hcl`, `Rules`, `Poller`, `Sniffer`,
  `Registry`, `Internal`, а также зарезервированные `Cluster` и `AdapterProxy`.
  Происхождение отображается в `RuntimeSource` провенанса и уточняет приоритет
  провода для нескольких видов (см. [`semantic-dali-commands.md`](semantic-dali-commands.md)).
- `target_adapter_id`, `bus_id` — экземпляр шины; `timestamp_ms` — монотонное время
  контроллера.
- `cluster_origin_id` / `adapter_proxy_origin_id` — зарезервированы под подавление
  петель будущих `I6`/`I7`; вне этих происхождений — `0`.
- `error` — только на пути отказа, в куче. `ErrorPayload` (код, текст до 64 байт,
  детали) едет по подтверждениям в HTTP; внутри событий используется
  `CompactErrorPayload` (код и текст до 32 байт).

## Вне шины

Экспорт и импорт конфигурации (`/api/v1/config/slices`) переносят байты слайсов
напрямую между HTTP и слайс-стором: слайс до 32 КиБ не проходит через 128-байтный
кадр. По шине едет только `RegistrySliceReloadCommand` — «перечитай», и оно доходит до
единственного писателя.

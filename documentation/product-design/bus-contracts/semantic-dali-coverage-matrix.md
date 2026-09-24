# Матрица покрытия семантических DALI-команд

Каждый продуктовый сценарий, который трогает DALI, отображается ровно на одно
семейство команд или на явное диагностическое исключение. Матрица отвечает на вопрос
«какой payload этот источник вправе опубликовать».

Границы: смысл команд — [`semantic-dali-commands.md`](semantic-dali-commands.md);
владельцы и дисциплины ответа — [`commands.md`](commands.md). Планируемые источники
(кластер, adapter proxy) — в [`../roadmap.md`](../roadmap.md).

| Источник | Сценарий | Payload |
|---|---|---|
| REST | target-state лампы | `DaliSetTargetStateCommand`, `scope = VirtualLamp`; непривязанная лампа — ничего |
| REST | target-state устройства | `DaliSetTargetStateCommand`, `scope = Short` |
| REST | target-state группы | `DaliSetTargetStateCommand`, `scope = Group` |
| REST | apply матрицы групп / сцены | `GroupApplyExecuteCommand` / `SceneApplyExecuteCommand` (ячейки публикует оркестратор) |
| REST | recall сцены | `DaliRecallSceneCommand`, `scope = Broadcast` (пустое тело) или `Group` |
| REST | запись атрибутов | `DaliWriteAttributesCommand` (`signals_operation = true`) |
| REST | discovery-run | `DaliDiscoverDevicesCommand` |
| REST | чтение атрибутов | `DaliReadAttributesCommand` (с пресетом банков) |
| REST | identify / смена адреса / замена / экспертный шаг | `DaliIdentifyDeviceCommand` / `DaliAddressingCommand` / `DaliReplaceDeviceCommand` / `DaliCommissioningStepCommand` |
| REST | устройства ввода: скан, коммиссионинг, identify | `Dali103ScanCommand`, `Dali103CommissionCommand`, `Dali103IdentifyCommand` |
| REST | устройства ввода: инстанс, индикация | `Dali103InstanceConfigureCommand`, `Dali103FeedbackConfigureCommand` |
| REST | применить политики ко всем | `PolicyApplyExecuteCommand` |
| REST | плановое переключение роли | `Dali103HandoverCommand` |
| REST | диагностика `/api/v1/dali/*` | `DaliCommandPayload` с `raw_mode` — **диагностическое исключение** |
| Home Assistant | свет лампы | `DaliSetTargetStateCommand`, `scope = VirtualLamp` |
| Home Assistant | свет группы | `DaliSetTargetStateCommand`, `scope = Group` |
| Home Assistant | выбор сцены | `DaliRecallSceneCommand`, `scope = Broadcast` |
| HCL | точка `level_mode = absolute` или `none` | `DaliSetTargetStateCommand`, `scope ∈ {Group, Broadcast}` (при `none` — только цвет) |
| HCL | точка `level_mode = last_active` | `DaliRecallLastActiveLevelCommand` на ту же цель + отдельная команда цвета |
| Правила | световые действия | `DaliSetTargetStateCommand` (лампа, группа, broadcast), `DaliStopFadeCommand` |
| Правила | сцены | `DaliRecallSceneCommand` (`Group` / `Broadcast`), `SceneApplyExecuteCommand` |
| Правила | индикация панелей | `Dali103FeedbackDriveCommand` |
| Поллер | фоновое чтение | `DaliReadAttributesCommand`, `scope = Short`, `Origin::Poller` (серии банков 202-207 — пресетом) |
| Поллер | health-probe сегмента | `DaliBusHealthProbeCommand` |
| Apply orchestrator | ячейки apply | `DaliProgramGroupMembershipCommand`, `DaliProgramSceneCommand` (`target = VirtualLamp`) |
| Apply orchestrator | ячейки политики | `DaliWriteAttributesCommand` (`signals_operation = false`) |
| Воркер арбитража | зонд владельца шины | `Dali103ArbitrationProbeCommand` |
| Проектор state-fanout | проекция фактов в реестр | `RegistryRuntimeUpdateCommand`, `RegistryLevelTransitionCommand` — DALI-команд не публикует |
| Sniffer translator | чужие кадры | только события (`DaliObservedFrameEvent`, вход Part 103) — команд не публикует |

`DaliProgramTarget = Short` допустим контрактом, но ни один документированный
продуктовый сценарий его сейчас не использует.

## Правило тестирования

Каждую строку покрывают хотя бы один BDD-сценарий, утверждающий опубликованный payload
и кадры провода (как — [`05`](../../architecture/05-testing-and-bdd.md)), и контрактный
тест сериализации. Сырой payload не появляется нигде, кроме диагностической строки.

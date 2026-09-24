# Модуль state-fanout

Проектор runtime-фактов: единственный продовый издатель `RegistryRuntimeUpdateCommand`.
Превращает типизированные факты о шине — наша команда исполнена, сцена вызвана, чужой
кадр увиден, устройство прочитано — в коммиты runtime-состояния, раскрывая группы,
broadcast и сцены по участникам.

Границы: как реестр сливает коммит — [`../../../architecture/06-registry-and-persistence.md`](../../../architecture/06-registry-and-persistence.md);
контракт RRUC — [`../../bus-contracts/commands.md`](../../bus-contracts/commands.md)
§Runtime-команды реестра; откуда берутся наблюдения —
[`../sniffer-translator/README.md`](../sniffer-translator/README.md). Код —
`projector_worker` в `dali2rust-fanout-runtime`.

## Входы и выходы

- **Потребляет**: `DaliTargetStateAppliedEvent`, `DaliSceneRecalledEvent`,
  `DaliObservedFrameEvent`, секцию `RuntimeStatus` из `DaliAttributesReadEvent` и
  `DaliAttributeReadOutcomesEvent` (устройство не ответило).
- **Публикует** только `RegistryRuntimeUpdateCommand` и
  `RegistryLevelTransitionCommand`, через `publish_required`. DALI-команд не
  публикует, в реестр напрямую не пишет, активацию сцены не синтезирует.
- **Читает** `ProjectorReadPort`: привязки, applied-членство групп, applied-строки
  сцен, capability ламп.

## Правила раскрытия

| Факт | Раскрытие |
|---|---|
| Наша команда по лампе или короткому адресу | одна запись с обеими идентичностями — один физический коммит |
| Наша команда или чужой кадр на группу | привязанные лампы — **applied**-члены группы |
| Broadcast | все привязанные лампы адаптера |
| Recall сцены (наш или чужой), broadcast | только **applied**-строки сцены; desired-only строки не проецируются |
| Recall сцены на группу | applied-строки сцены, чьи лампы — applied-члены группы |
| Чужой кадр по короткому адресу | лампа, привязанная к адресу, и само устройство |
| Арк-команда без уровня (`LevelTransitionObserved`) | `RegistryLevelTransitionCommand` на каждого участника; уровень считает реестр |
| Секция `RuntimeStatus` чтения | один коммит устройства |
| Исход чтения «устройство не ответило» | наблюдение отсутствия (`device_absent`) без уровня и без `last_seen_ms` |

- **Цветовой гейт** при раскрытии на разнородных участников —
  [`../../../architecture/06-registry-and-persistence.md`](../../../architecture/06-registry-and-persistence.md)
  §Colour capability gate; capability участника берётся из вида лампы, то есть с учётом
  override устройства.
- **Коалесинг**: подряд идущие наблюдения сниффера на одну цель
  `(адаптер, scope, идентичность)` сворачиваются в одно финальное — чужой фейд не
  заливает инбокс реестра. Любой другой факт сначала сбрасывает буфер, поэтому
  порядок относительно наших фактов сохраняется.
- **Пейсинг**: раскрытие публикуется пачками по восемь с короткой паузой и делит один
  бюджет ретраев на всё раскрытие (`ADR-007`, `ADR-021`).

## Корреляция и источник

- Какие факты несут корреляцию запроса, а какие `CORRELATION_NONE`, и как проброшенная
  монотонная метка упорядочивает наблюдения —
  [`../../../architecture/06-registry-and-persistence.md`](../../../architecture/06-registry-and-persistence.md)
  §Update path и §Merge rules. Метка applied-факта — момент применения на проводе;
  отсутствие не штампуется.
- Источник коммита команды — из её `Origin` (`Api`, `Mqtt`, `Hcl`, `Rules`);
  наблюдения сниффера — `Sniffer`; секция `RuntimeStatus` — `Readback`: чтение ничего
  не командует, и HCL не должен принимать его за ручное вмешательство; отсутствие
  устройства — `Poller`.

## `last_dapc_source`

Проектор — единственный, кто его выставляет:

| Факт с DAPC | Значение |
|---|---|
| Наша команда по лампе или короткому адресу | `unknown` (`null` в JSON) |
| Наша или чужая команда на группу или broadcast | `group` |
| Чужой кадр по короткому адресу | `sniffer` |
| Recall сцены | `scene` |

Факт без DAPC (только цвет, только статус) передаёт `None`, и реестр сохраняет
прежнее значение. HCL различает источники по источнику коммита, а не по этому полю.

## Состояние и тесты

Прочного состояния нет, только буфер коалесинга. Чёрный ящик — `SYS-210..215` и
групповой recall (`tests/dali2rust-bdd/features/system/integration-fanout.feature`):
источники API и сниффер. Раскрытие, гейт цвета, `last_dapc_source`, коалесинг —
крейт-тесты `projector_flow.rs` и `sniffer_translator_flow.rs`: канальный уровень
шины чёрному ящику недоступен.

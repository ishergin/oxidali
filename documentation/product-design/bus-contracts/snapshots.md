# Read-модели и персистентные слайсы

Что клиент читает как модель состояния и что из состояния переживает перезагрузку:
состав слайсов конфигурации, что каждый несёт и чего не несёт никто.

Границы: механика слайс-стора, версии и гидрация —
[`../../architecture/06-registry-and-persistence.md`](../../architecture/06-registry-and-persistence.md);
что переносится между контроллерами — [`../rest-api/resources/config-transfer.md`](../rest-api/resources/config-transfer.md);
гарантии чтения — [`../runtime-modules/registry/read-port.md`](../runtime-modules/registry/read-port.md);
стабильность полей API — [`../rest-api/stability-and-versioning.md`](../rest-api/stability-and-versioning.md).

## Read-модели

Реестр отдаёт состояние через read-порты; версионированных снапшотов runtime нет.
REST DTO — проекции поверх реестра ([`../rest-api/README.md`](../rest-api/README.md)).
Статистика — read-модель поверх счётчиков в памяти
([`../runtime-modules/stats/README.md`](../runtime-modules/stats/README.md)).

## Слайсы конфигурации

Слайс именуется `SliceKey` и несёт свою версию формата
([06](../../architecture/06-registry-and-persistence.md) §Versions). Контроллер-глобальные
слайсы дописываются в конец раскладки, поэтому новый глобальный слайс не сдвигает уже
записанные пер-адаптерные.

| Слайс | Что несёт |
|---|---|
| `Adapters` | имя и `enabled` каждого адаптера |
| `VirtualLamps { adapter }` | имя лампы, гейт Home Assistant, привязка |
| `PhysicalDeviceBank { adapter, bank }` | по четыре устройства: метаданные оператора, override, random address, персистентные секции атрибутов, сводка банков памяти, липкие доказательства (типы, capability, Tc-диапазон), пер-устройственные разрешения DT8 |
| `PhysicalDevices { adapter }` | прежняя форма целиком на адаптер; читается один раз, пока банки пусты |
| `Groups { adapter }` | метаданные 16 групп и desired-матрица членства с признаком посева |
| `Scene { adapter, scene }` | метаданные сцены и её desired-матрица с признаком посева — по слайсу на сцену |
| `ControllerSettings` | часовой пояс; пишется вне реестра |
| `HclSchedules` | все расписания HCL |
| `PollerSettings` | настройки поллера |
| `InputDevices { bank }` | устройства Part 103: идентичность, метаданные, компактное резюме инстансов (тип и конфигурация событий) |
| `Rules { bank }` | исходник документа правил байт-в-байт и манифест (язык, CRC, биты включения); пишет rules runtime |
| `HomeAssistantSettings` | брокер, пароль, префиксы, `controller_id`, гейты экспонирования |
| `DaliSettings` | политика DT8, `applicationActive`, собственный адрес контроллера как control device |
| `RedundancySettings` | роль, период зонда, порог захвата, адрес пира |
| `Policies` | `systemFailureLevel` / `powerOnLevel` установки, применение к новому гиру |

## Что не персистится

- Runtime ламп: питание, уровень, цвет, статус, отказы, `last_dapc_source`, тень
  `lastActiveLevel`.
- `applied` групп и сцен: выводится из readback'ов (маска членства и уровни сцен
  персистятся как доказательства устройства), цвет сцены — из наблюдения, которое
  волатильно.
- Живые измерения и данные, которые восстанавливает одно чтение: банки 202-207,
  банк 0 выше `0x1A`, расширение банка 1 (Part 251), цвета сцен
  ([`dali-attribute-taxonomy.md`](dali-attribute-taxonomy.md)).
- Таймеры инстансов Part 103 (их может поменять локальная настройка устройства),
  runtime инстансов, конфигурация индикации Part 332, переменные и таймеры правил,
  флаги override HCL, «группой командовали» плиток Home Assistant.
- Операции, счётчики, статистика, staged-серии чанковых записей.

## Загрузка

Гидрация и её порядок — [06](../../architecture/06-registry-and-persistence.md)
§Hydration; итог по каждому слайсу — `PersistenceLoadResultEvent`, перечитка после
импорта или репликации — `RegistrySliceReloadCommand` и `RegistrySliceReloadedEvent`
([`events.md`](events.md)).

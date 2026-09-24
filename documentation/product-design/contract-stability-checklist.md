# Чеклист стабильности контрактов

Проверки перед реализацией или слиянием продуктового поведения, которое меняет
публичный или шинный контракт.

Границы: правила версионирования API — [`rest-api/stability-and-versioning.md`](rest-api/stability-and-versioning.md);
механика объявления payload'ов — [`../architecture/11-extension-recipes.md`](../architecture/11-extension-recipes.md);
BDD-конвенции — [`../architecture/05-testing-and-bdd.md`](../architecture/05-testing-and-bdd.md).

## API

- [ ] Маршрут описан в ресурсном документе REST, запрос и ответ — тоже.
- [ ] `PATCH` — `application/merge-patch+json`; поведение `null` определено для каждого
      nullable-поля.
- [ ] Неизвестное поле — `400 unknown_field`; runtime-поле в теле —
      `422 unsupported_field`.
- [ ] Мутирующий маршрут отображается на документированное семейство команд
      ([`bus-contracts/commands.md`](bus-contracts/commands.md)) и его дисциплину ответа.
- [ ] Секреты только на запись: в read-DTO — признак (`broker_password_set`), а не
      значение.
- [ ] Ошибка — `ErrorDto`; новый `ErrorCode` дописан в конец перечисления.
- [ ] Ответ на чтение укладывается в стек задачи httpd: ресурс, а не дамп; потолок
      размера DTO не поднят ради нового поля.
- [ ] **Удалённое поле записано ниже**: снаружи это ломающее изменение, а внутри оно
      может разойтись в нескольких местах сразу.

Снятые поля:

| Поле | Что теперь | Что зарезервировано |
|---|---|---|
| `PollerSettingsDto.max_concurrent` | `400 unknown_field`; в полёте всегда одно чтение (`ADR-009`) | значение `4` маски `PollerSettingsUpdateCommand` |
| `declared_type` / `declared_color_mode` виртуальной лампы | `400 unknown_field`; лампа наследует тип и цвет от устройства | — |
| `duration_ms` identify | `422 unsupported_field`; окно опознания принадлежит гиру | — |

## Общее состояние

- [ ] Тип устройства, цветовой режим, питание, уровень, статус — общие перечисления и
      структуры ([`bus-contracts/shared-state-contracts.md`](bus-contracts/shared-state-contracts.md)).
- [ ] Новый DTO ссылается на `LightSetpoint` / `RuntimeObservation` композицией и не
      дублирует их поля ([`rest-api/contracts/state-contracts.md`](rest-api/contracts/state-contracts.md)).
- [ ] Runtime-DTO — `RuntimeStateContract`; тело target-state — `TargetStateRequestDto`
      без полей `RuntimeObservation`; строка сцены — `SceneRow` без `transition` и
      runtime-полей.
- [ ] Capability выводятся одинаково на всех поверхностях; источник override и
      effective-значение явны.

## Шина

- [ ] Payload объявлен в `declare_bus_payloads!` с худшим `budget`-сэмплом; `max = N` —
      только с обоснованием.
- [ ] Вариант дописан в **конец** юниона, снапшот `FROZEN_*_ORDER` расширен.
- [ ] У команды ровно один владелец в таблице диспетча; у события есть потребитель или
      запись в `OBSERVED_ONLY_EVENTS` с причиной (`bus_payload_ownership`).
- [ ] Событие, единственное несущее факт, публикуется через `publish_required` и
      объявлено в `<PUBLISHER>_REQUIRED_EVENTS`.
- [ ] Payload помещается в 128 байт или едет чанковой серией со staging'ом и
      закрывающей скобкой.
- [ ] Продуктовый DALI-сценарий публикует семантическую команду; сырой путь — только
      диагностика ([`bus-contracts/semantic-dali-commands.md`](bus-contracts/semantic-dali-commands.md)).
- [ ] Новый источник DALI-команд добавлен в
      [`bus-contracts/semantic-dali-coverage-matrix.md`](bus-contracts/semantic-dali-coverage-matrix.md).
- [ ] Новый вид команды DALI имеет строку в таблице приоритетов провода.
- [ ] `202`-маршрут открывает операцию, а исполнитель её закрывает сигналом.
- [ ] Статистика остаётся read-моделью `GET /api/v1/stats` и на шину не публикуется.

## Реестр

- [ ] Мутация применяется только registry worker'ом; чтение — через read-порт.
- [ ] Runtime не персистится и меняется только через `RegistryRuntimeUpdateCommand`.
- [ ] Applied групп и сцен меняется только из readback'ов, desired — только командами.
- [ ] Доказательства (чтение атрибутов, запись, итог скана, банк памяти) применяются
      прямым путём реестра, а не через runtime-команду.
- [ ] Эффективная мутация публикует своё событие изменения.

## Персист

- [ ] Новое поле в персистентной записи — бамп версии **только** своего слайса, с
      перечнем того, что бамп сбрасывает; для слайса физических устройств это
      перепривязка всех ламп
      ([`../architecture/06-registry-and-persistence.md`](../architecture/06-registry-and-persistence.md)
      §Versions).
- [ ] Данные, которые восстанавливает одно чтение, лежат рядом с `attributes` и
      версию не двигают.
- [ ] Новый контроллер-глобальный слайс дописан в конец раскладки.

## Атрибуты DALI

- [ ] Атрибут отнесён к классу и секции
      ([`bus-contracts/dali-attribute-taxonomy.md`](bus-contracts/dali-attribute-taxonomy.md)).
- [ ] Чтение и запись — семантическими командами; запись разрешена явно.
- [ ] Обнаруженное поле через общий PATCH не пишется.

## BDD

- [ ] Есть сценарии успеха и отказа валидации.
- [ ] Утверждается тип опубликованного payload'а и, для DALI, кадры на мок-транспорте.
- [ ] Утверждается чтение состояния обратно через REST.
- [ ] Видимые снаружи эффекты MQTT и WebSocket утверждаются.
- [ ] Новый воркер покрыт по переполнению и отставанию на своём слое.
- [ ] Префикс ID зарегистрирован в [`bdd/ids-registry.md`](bdd/ids-registry.md).

## Home Assistant

- [ ] Есть фикстуры discovery и состояния.
- [ ] Топик команды отображается на семантическую команду.
- [ ] `unique_id` стабилен при перепривязке.
- [ ] Смена capability и смена настроек моста приводят к переанонсу.

## WebSocket

- [ ] Payload канала — проекция того же DTO, что REST.
- [ ] Новый вид события объявлен в наборе проецируемых и в таблице проекции.
- [ ] Медленный клиент не блокирует ни шину, ни других клиентов.

## Статус

- [ ] [`status.md`](status.md) и [`roadmap.md`](roadmap.md) отражают сделанное и
      оставшееся; закрытое из дорожной карты убрано.

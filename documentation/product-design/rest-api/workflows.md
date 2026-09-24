# Сквозные сценарии REST

Цепочки, которые проходят через несколько ресурсов и воркеров: от запроса до коммита
в реестре и публикации наружу.

**Границы:** маршруты, ответы и ошибки — в документах ресурсов
([`README.md`](README.md)); поведение воркеров — в
[`../runtime-modules/`](../runtime-modules/); шинные payload'ы — в
[`../bus-contracts/`](../bus-contracts/); сквозные инварианты (один писатель реестра,
runtime только через проектор) — [`../glossary-and-invariants.md`](../glossary-and-invariants.md).

## Инвариант всех цепочек

Внешнее состояние (REST read-модель, WebSocket, MQTT) меняется только после коммита
в реестре; коммит порождает `RuntimeStateChangedEvent` или `*ChangedEvent`. DALI worker
публикует типизированные факты (что применено, что прочитано), а не обновления реестра.

## Target-state

1. `PUT …/target-state` на виртуальную лампу, группу или физическое устройство; тело
   валидируется против effective capabilities цели
   ([`contracts/state-contracts.md`](contracts/state-contracts.md) §Тело target-state).
   Что особенного у лампы — no-op без привязки и адрес, разрешаемый в момент исполнения,
   — [`resources/virtual-lamps.md`](resources/virtual-lamps.md).
2. Публикуется `DaliSetTargetStateCommand` со scope `VirtualLamp` / `Group` / `Short`.
3. DALI worker коалесцирует инбокс по цели
   ([03](../../architecture/03-bus-and-backpressure.md) §Coalescing), исполняет
   команду на проводе и публикует applied-факт и подтверждение; handler отвечает по
   дисциплине синхронного подтверждения.
4. Проектор state-fanout разворачивает факт: группу — по **applied**-членству,
   broadcast — по адаптеру, адрес — в привязанную лампу; публикует
   `RegistryRuntimeUpdateCommand`. Реестр коммитит и публикует
   `RuntimeStateChangedEvent`; WS и MQTT реагируют на него.

Чужие кадры на шине (другой мастер, панель) проходят тот же путь с шага 4: сниффер →
транслятор → проектор, с `value_source: sniffer`.

## Матрицы групп и сцен → apply

Общее для `POST …/groups/apply` и `POST …/scenes/{id}/apply`; своё у каждого — в
[`resources/groups.md`](resources/groups.md) и [`resources/scenes.md`](resources/scenes.md).

1. `PATCH` / `PUT` матрицы меняет только `desired` — чанковой записью `202`
   `config_write` ([`stability-and-versioning.md`](stability-and-versioning.md)
   §Дисциплины ответа).
2. `POST …/apply` сравнивает `desired` и `applied` лишь как быструю проверку: пустой
   diff — `200` с актуальной матрицей и без операции; иначе — **одна**
   `*ApplyExecuteCommand` и `202`.
3. Apply-оркестратор перечитывает авторитетный снимок реестра, сам разворачивает diff
   и публикует по одной программирующей команде на ячейку, дожидаясь исхода
   предыдущей ([`apply-orchestrator`](../runtime-modules/apply-orchestrator/README.md)).
   Непривязанные строки пропускаются без провода.
4. После каждой ячейки worker перечитывает прибор и публикует факт; реестр проецирует
   `applied` из этого readback, а не из команды.
5. Частичный отказ не откатывает успешные ячейки; повторный apply программирует
   только оставшийся diff. Итоги по ячейкам — `result` операции
   ([`resources/operations.md`](resources/operations.md)).

## Discovery

`POST …/discovery-runs` (`202`, `discovery`) → `DaliDiscoverDevicesCommand`; worker
публикует прогресс по каждому подтверждённому прибору, и реестр создаёт или обновляет
запись по каждому событию прогресса — что значит «подтверждённый», говорит
[`resources/physical-devices.md`](resources/physical-devices.md). Если политики
отказоустойчивости настроены на запись при обнаружении, успешный скан запускает одну
операцию `policy_apply` ([`resources/policies.md`](resources/policies.md)).

## Чтение атрибутов

`POST …/attribute-reads` (`202`, `attribute_read`) → одна `DaliReadAttributesCommand`.
Worker читает запрошенные группы, затем (если задан пресет) банки памяти, и публикует
факты чтения под одной корреляцией. Runtime-секция уходит в реестр через проектор;
остальные секции и банки реестр применяет как evidence. Итоги по каждой группе —
`attribute_read_outcomes` операции.

## Прочие цепочки

- Вызов сцены — нативный кадр и проекция по applied-строкам:
  [`resources/scenes.md`](resources/scenes.md) §`POST …/recall`.
- Commissioning (identify, смена адреса, замена прибора) —
  [`resources/commissioning.md`](resources/commissioning.md).
- HCL: расписание — конфигурация в реестре; планировщик публикует семантические
  команды с `origin = Hcl`, а ручное управление приостанавливает его по цели —
  [`resources/hcl.md`](resources/hcl.md),
  [`hcl-scheduler`](../runtime-modules/hcl-scheduler/README.md).
- Переанонс Home Assistant (`POST /settings/home-assistant/discovery-publish`) —
  [`resources/settings-home-assistant.md`](resources/settings-home-assistant.md),
  [`mqtt-home-assistant`](../runtime-modules/mqtt-home-assistant/README.md).

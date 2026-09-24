# Модуль API gateway

HTTP-слой контроллера для web UI, внешних клиентов и диагностики: валидирует запрос,
публикует типизированную команду и отвечает. Состояния продукта не держит.

Границы: публичный контракт маршрутов — [`../../rest-api/README.md`](../../rest-api/README.md);
мост подтверждений и корреляция — [`../../../architecture/04-contracts-and-api-bridge.md`](../../../architecture/04-contracts-and-api-bridge.md);
единственная задача httpd и её стек — [`../../../architecture/02-runtime-and-threading.md`](../../../architecture/02-runtime-and-threading.md).
Код — `dali2rust-api` (хендлеры, `bus_codec`, JSON), подключение — композиция в
`dali2rust-adapters`.

## Входы и выходы

- **Читает** реестр через read-порты и мосты `*HttpState`, таблицу операций через
  `OperationReadPort`.
- **Публикует** команды реестра, операций, семантические DALI-команды и Part 103 —
  только продакшн-билдерами из `dali2rust-contracts`. Сырой кадр публикуют лишь
  диагностические маршруты `/api/v1/dali/*`.
- **Не делает**: не мутирует реестр, не вызывает транспорт DALI, не собирает опкоды,
  не разворачивает массовое применение в поток команд, не разрешает привязку лампы
  (это делает исполнитель в момент исполнения).

## Форма хендлера

`validate → build → publish → respond` ([`CLAUDE.md`](../../../../CLAUDE.md), золотое
правило 3).

- **Валидация — до шины.** Коды отказов и строгий merge-patch —
  [`../../rest-api/stability-and-versioning.md`](../../rest-api/stability-and-versioning.md);
  путевой параметр разбирается своим декодером, не декодером query —
  [`../../../architecture/04-contracts-and-api-bridge.md`](../../../architecture/04-contracts-and-api-bridge.md)
  §JSON boundaries.
- **Дисциплина ответа — по виду команды**
  ([`../../bus-contracts/commands.md`](../../bus-contracts/commands.md) §Дисциплины
  ответа). Все хендлеры делят одну задачу httpd: один дедлайн на запрос и уход долгой
  работы в `202` —
  [`../../../architecture/02-runtime-and-threading.md`](../../../architecture/02-runtime-and-threading.md)
  §The HTTP task.

## Чтение

Ответ на чтение строится на стеке задачи httpd, поэтому ресурс режется по частям, а не
отдаётся дампом —
[`../../../architecture/07-memory-and-cores.md`](../../../architecture/07-memory-and-cores.md)
§The httpd stack is a budget.

## Применение, recall, роль

- Apply групп, сцен и политик — одна execute-команда и `202`; развёртку и пейсинг делает
  [`../apply-orchestrator/README.md`](../apply-orchestrator/README.md) (`ADR-007`).
  Быстрый путь пустого diff в шлюзе — тот же общий хелпер `dali2rust-domain`, что у
  оркестратора. Ответы — ресурсы [`groups`](../../rest-api/resources/groups.md),
  [`scenes`](../../rest-api/resources/scenes.md),
  [`policies`](../../rest-api/resources/policies.md).
- Recall сцены — только нативный `DaliRecallSceneCommand` со строгим разбором тела
  ([`scenes`](../../rest-api/resources/scenes.md) §`POST …/recall`).
- Заголовок роли ставит роутер в одном месте, отказ записи на стэндбае приходит из
  гейта `DaliWorker` ([`redundancy`](../../rest-api/resources/redundancy.md)).

## Тесты

- `GET` не публикует команд; мутирующий маршрут утверждает тип опубликованного
  payload'а; диагностический сырой путь — единственное исключение.
- Применение: `200` + матрица на пустой diff, `202` + одна execute-команда на непустой;
  пейсинг доказывает оркестратор.
- Контракты ответов — DTO и contract-тесты `dali2rust-api`; поведение — чёрный ящик
  BDD в `tests/dali2rust-bdd/features/`.

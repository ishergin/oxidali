# Продуктовый дизайн dali2rust

Пакет продуктового дизайна контроллера DALI для Home Assistant: публичные контракты,
модули, UI, статус и открытая работа. Написан по-русски — единственное языковое
исключение репозитория ([`../../CLAUDE.md`](../../CLAUDE.md), золотое правило 5);
имена типов, JSON-ключи и сценарии Gherkin остаются английскими.

Границы: как система устроена в коде — [`../architecture/README.md`](../architecture/README.md);
правила работы с репозиторием — [`../../CLAUDE.md`](../../CLAUDE.md); исполняемый
BDD-канон — `tests/dali2rust-bdd/features/`.

## Карта

| Путь | Что там |
|---|---|
| [`glossary-and-invariants.md`](glossary-and-invariants.md) | Термины пакета и сквозные инварианты |
| [`status.md`](status.md) | Что построено, по стадиям |
| [`roadmap.md`](roadmap.md) | Открытая работа: стадии, остатки, технический долг, шаблон стадии |
| [`known-issues.md`](known-issues.md) | Открытые дефекты и отклонения |
| [`issue-ids-registry.md`](issue-ids-registry.md) | Единственная точка выделения номеров `ISSUE-NN` |
| [`contract-stability-checklist.md`](contract-stability-checklist.md) | Проверки перед слиянием изменения контракта |
| [`rest-api/`](rest-api/README.md) | REST API: ресурсы, DTO, ошибки, workflows |
| [`bus-contracts/`](bus-contracts/README.md) | Команды и события шины, общие типы состояния, атрибуты, слайсы |
| [`runtime-modules/`](runtime-modules/README.md) | Модули контроллера: что делают и за что отвечают |
| [`web-ui/`](web-ui/README.md) | Экраны встроенного web UI |
| [`bdd/`](bdd/README.md) | Реестр префиксов BDD-ID |

## Сквозной поток

Кто единственный владелец состояния и провода —
[`glossary-and-invariants.md`](glossary-and-invariants.md) §Инварианты; путь запроса от
маршрута до коммита и публикации наружу — [`rest-api/workflows.md`](rest-api/workflows.md).

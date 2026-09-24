# Ресурс: Diagnostic DALI

Низкоуровневый интерфейс отправки 16-битных кадров DALI — для инструментов
разработки и сервиса. Продуктовый путь им не пользуется.

**Границы:** продуктовые команды — типизированные ресурсы (target-state,
commissioning, атрибуты); правило «продукт не собирает опкоды» и приоритет кадров
этого пути —
[`09-dali-protocol-rules.md`](../../../architecture/09-dali-protocol-rules.md). BDD —
[`diagnostic`](../../../../tests/dali2rust-bdd/features/diagnostic/).

## Маршруты

| Метод | Путь | Назначение |
|---|---|---|
| `POST` | `/api/v1/dali/command` | Валидированная команда: `wire_address` + `command` декодируются доменом |
| `POST` | `/api/v1/dali/level` | Уровень arc power на `wire_address` |
| `POST` | `/api/v1/dali/raw` | Сырой 16-битный кадр без доменного декодирования |

- Поле адреса — `wire_address`, **байт адреса на проводе**, а не short address.
  `repeat_count` — повтор для команд send-twice.
- Каждая ручка публикует диагностический `DaliCommandPayload` (для `raw` — в сыром
  режиме) и ждёт подтверждения — синхронная дисциплина. Семантических команд не
  публикует, runtime-состояние реестра не меняет, операций не создаёт.

## Ответ

Не ресурсный DTO, а тело подтверждения — одинаковое у трёх ручек:

- `success`, `backward_frame` (`0`, если обратного кадра не было);
- `error` — **статус доставки** плоской строкой (`delivery_rejected` /
  `execution_failed` / `timeout`, `null` при успехе), при наличии — `error_code`
  (продуктовый код) и `message`;
- `backward_violation: true` — в окне ответа пришёл нарушающий кадр.

**`backward_frame: 0` при `success: false` — не ответ `0x00`, а отсутствие ответа.**
Читать байт, не проверив `success`, значит сфабриковать ответ из тишины.

Отказы самого конверта — обычные `{"error": …}`: `503 commands_ingress_overload` /
`confirmation_slots_exhausted`, `504 confirmation_timeout`; тело не JSON — `400
invalid_json`; пара «адрес + команда», которую домен не декодирует (для `command` и
`level`), — `400 invalid_dali_command`.

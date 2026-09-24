# Ресурс: Controller

Сводка контроллера, проверка живости и часы.

**Границы:** настройки подсистем — ресурсы `settings-*`; роль в паре
отказоустойчивости — [`redundancy.md`](redundancy.md); счётчики —
[`stats.md`](stats.md) и [`diagnostics.md`](diagnostics.md).

## Маршруты

| Метод | Путь | Назначение |
|---|---|---|
| `GET` | `/api/v1/controller` | Сводка контроллера |
| `GET` | `/api/v1/health` | Живость, версия, роль |
| `GET` | `/api/v1/time` | Состояние часов |
| `PUT` | `/api/v1/time` | Задать время и/или часовой пояс |

Все чтения ничего не публикуют на шину. BDD:
[`adapters`](../../../../tests/dali2rust-bdd/features/adapters/) (сводка),
[`system`](../../../../tests/dali2rust-bdd/features/system/) (health),
[`hcl`](../../../../tests/dali2rust-bdd/features/hcl/) (часы).

## `GET /controller`

- `firmware_version` — версия образа, та же, что `version` у `/health`; формат и где
  ещё она видна — [`10`](../../../architecture/10-build-release-and-tooling.md)
  §Firmware version.
- `target_mcu` — `esp32p4` для прошивки и `host` для хост-сборки (выводится из
  архитектуры, а не записан литералом).
- `uptime_ms` — от старта HTTP-поверхности, по тем же часам, что `/health`.
- `home_assistant` — `enabled` и `broker_url` из настроек, `connected` — из самого моста:
  включённый мост, которому брокер отказывает, виден как неподключённый.
- `adapter_count` — число DALI-адаптеров, фиксируется при загрузке.
- `controller_id`, `network` (`hostname`, `ip`) и `cluster` сейчас не наполнены:
  отдаются константы (`local`, пустые строки, `enabled: false`). Идентичность для Home
  Assistant — `controller_id` в [`settings-home-assistant.md`](settings-home-assistant.md).

## `GET /health`

`status` (`ok` / `degraded` / `down`), `uptime_seconds`, `version` и — когда в сборке
есть redundancy — `role` (`active` / `standby`). Роль продублирована в теле, хотя есть
заголовок `X-Dali2rust-Role`: системы мониторинга сохраняют разобранный JSON и
отбрасывают заголовки.

## `GET` / `PUT /time`

Расписания HCL и правила по времени суток работают только по привязанным часам: пока
`synced: false`, планировщик ничего не публикует. Якорь — SNTP; `PUT` — ручной якорь
на случай, когда SNTP недоступен, и единственный способ задать зону.

- `GET`: `synced`, `unix_ms` (нет, пока часы не привязаны — чтобы клиент не принял
  миллисекунды от загрузки за реальное время), `source`, `timezone` (строка POSIX TZ),
  `local_minutes` (минуты локального времени от полуночи — шкала точек HCL),
  `utc_offset_minutes`.
- `PUT {"unix_ms"?, "timezone"?}` — оба поля необязательны и независимы; зона
  сохраняется во флеше. Ответ — то же, что `GET`. Завершённая синхронизация SNTP затем
  побеждает ручной якорь.
- Ошибки: неправдоподобное время или неразбираемая зона — `422 invalid_value`.

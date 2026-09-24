# Ресурс: Settings — Poller

Настройки фонового опроса известных приборов: включение, период, что читать.

**Границы:** поведение поллера (окно тишины, бюджет провода, cooldown, health-probe) —
[`../../runtime-modules/poller/README.md`](../../runtime-modules/poller/README.md);
его счётчики — блок `poller` в [`diagnostics.md`](diagnostics.md). BDD —
[`settings_poller`](../../../../tests/dali2rust-bdd/features/settings_poller/),
[`poller`](../../../../tests/dali2rust-bdd/features/poller/).

## Маршруты

| Метод | Путь | Ответ |
|---|---|---|
| `GET` | `/api/v1/settings/poller` | `200` |
| `PATCH` | `/api/v1/settings/poller` | `200` — применённые настройки |

Настройки controller-global; `PATCH` — запись с read-after-write, поллер подхватывает
изменение без перезапуска. Настройки персистятся своим слайсом.

## Поля

| Поле | По умолчанию | Смысл |
|---|---|---|
| `enabled` | `false` | Без явного включения контроллер шину не опрашивает. |
| `interval_ms` | `5000` | `200..3600000`. **Пол, а не обещание**: период растягивает бюджет провода фона ([поллер](../../runtime-modules/poller/README.md)); растяжение видно в счётчике `poller.duty_deferred`. |
| `attribute_groups_default` | `["runtime_status"]` | Что читать у каждого прибора; непустой список групп ([`../contracts/enums.md`](../contracts/enums.md)). |
| `include_dt8_color` | `true` | Добавлять группу `dt8_color` только приборам с effective типом DT8 (явное `dt8_color` в списке выше добавляет её всем). |
| `include_energy` | `false` | Читать банки DiiA Part 252 у приборов, объявивших тип 51. |
| `include_diagnostics` | `false` | Читать банки Part 253 у приборов, объявивших тип 52. |
| `skip_unbound_virtual_lamps` | `true` | Не опрашивать приборы без привязанной виртуальной лампы. |

- Серии банков энергии и диагностики, их гейт по объявленному типу и интервалы — у
  [поллера](../../runtime-modules/poller/README.md); частоты не настраиваются. Два
  выключателя раздельны, потому что стоят по-разному.
- `enabled` и `interval_ms` управляют и широковещательным health-probe; своей
  настройки у него нет.
- Сознательно отсутствуют: ограничение параллельности (в полёте всегда одно чтение),
  поля backoff (политика фиксирована), пресет банков памяти (банки 0 и 1 поллер не
  читает, банки Part 252/253 — только сериями выше), исключения отдельных приборов
  (единственный фильтр — `skip_unbound_virtual_lamps`).

## Ошибки

| Случай | Ответ |
|---|---|
| `interval_ms` вне диапазона, пустой `attribute_groups_default`, значение неверного типа | `422 invalid_value` |
| Неизвестная группа атрибутов | `422 invalid_enum` |
| Неизвестное поле (в том числе любые `backoff_*`, `max_concurrent`) | `400 unknown_field` |

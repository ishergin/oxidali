# Экран: Settings → DALI

Controller-global поведение на шине (`#/settings/dali`), рядом с Poller, Home Assistant
и Redundancy.

**Границы:** контракт и смысл полей —
[`../rest-api/resources/settings-dali.md`](../rest-api/resources/settings-dali.md);
поустройственные исключения — карточка устройства; роль в паре — экран
[`redundancy.md`](redundancy.md); карточка экрана —
`web/design-system/screens/settings-dali.html`.

Вопрос «делать ли это вообще» у оператора один на установку, поэтому и выключатель
один; поустройственный флаг на карточке устройства прячется, пока прибор не прочитан, и
при десятках приборов отвечать на этот вопрос обходом карточек нельзя. Исключение
«кроме этой фикстуры» остаётся на карточке устройства и здесь не дублируется.

- **Colour**: «Restore DT8 auto activation» и «Assert RGBWAF channel control» —
  переключатели с подписью следствия, а не механики («без этого запись принимается, а
  цвет не двигается»).
- **Application controller**: «Active» (`application_active`) и собственный адрес
  control device (пусто — адреса нет).
- `warnbar` висит на **необычном** выборе, а не на обычном: на выключенном
  восстановлении auto activation, на выключенном утверждении RGBWAF control и на
  пассивном контроллере. Чем грозит каждый из них —
  [`../rest-api/resources/settings-dali.md`](../rest-api/resources/settings-dali.md)
  §Поля.

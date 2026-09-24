pub const SECTION7_ONE_BUTTON: &str = r#"
rule "коридор: вкл/выкл" {
  when input(dev=3, inst=0) is short_press
  do   lamp("коридор").toggle()
}

rule "коридор: диммирование" {
  when input(dev=3, inst=0) is long_press_repeat
  do   lamp("коридор").dim_hold(+60)      # 60 шагов в секунду удержания
}

rule "коридор: полный свет" {
  when input(dev=3, inst=0) is double_press
  do   lamp("коридор").on(level=254)
}
"#;

pub const SECTION7_NIGHT: &str = r#"
def "ночной контур" {
  scene(13).recall(group("ночь"))         # 30/254 и 2200 K одним кадром
  timer("ночь-выкл").cancel()
}

rule "ночь: вход" {
  when input(dev=5, inst=0) becomes occupied
  when input(dev=3, inst=0) is short_press
  if   time in 23:00 .. sunrise
  do   call("ночной контур")
}

rule "ночь: погашение" {
  when input(dev=5, inst=0) becomes vacant
  if   time in 23:00 .. sunrise
  do   group("ночь").level(10)   # притухли: «сейчас выключусь»
       timer("ночь-выкл").start(30s)
}

rule "ночь: выключение" {
  when timer("ночь-выкл") fires
  do   group("ночь").off()
       hcl.resume(group("ночь"))
}
"#;

pub const SECTION7_AWAY: &str = r#"
rule "ушёл" {
  when input(dev=3, inst=2) is long_press_start
  do   broadcast.off()
       var("режим").set("нет дома")
       mqtt.publish("dali2rust/mode", "away", retain=true)
       after 5m do { hcl.resume(broadcast) }
}
"#;

pub const SECTION7_BOOT: &str = r#"
rule "boot: режим по умолчанию" {
  when controller starts
  when controller becomes active
  do   var("режим").set("обычный")
       panel_select(group=4, selected=1)
}
"#;

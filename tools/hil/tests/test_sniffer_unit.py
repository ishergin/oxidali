from hil.sniffer import parse_line

TOPIC = "/wb-dali/wb-dali_19_bus_1/bus_monitor"


def test_a_stamped_monitor_line_is_read_as_a_monitor_line():
    entry = parse_line("1761465922.796 " + TOPIC
                       + " 08:05:22.796 << fffe3d FF24.QueryApplicationControlEnabled (fc: 4204)")
    assert entry["kind"] == "monitor" and entry["direction"] == "rx"
    assert entry["fc"] == 4204 and entry["text"].startswith("fffe3d")


def test_a_stamped_request_line_keeps_its_lunatone_mark():
    entry = parse_line("1761465925.882 " + TOPIC
                       + " 08:08:25.882 >> 0b90 QueryStatus(A5) (from lunatone) - 00 0")
    assert entry["direction"] == "tx" and entry["lunatone"]


def test_an_unstamped_monitor_line_still_reads():
    entry = parse_line("1761465922.796 " + TOPIC + " <<   a372 FF16 DTR0(114) (fc: 29786)")
    assert entry["direction"] == "rx" and entry["fc"] == 29786

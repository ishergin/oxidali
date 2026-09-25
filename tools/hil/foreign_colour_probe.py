#!/usr/bin/env python3
import sys
import time

sys.path.insert(0, ".")

from hil import config as cfg
from hil.api import Client
from hil.foreign import ForeignMaster

LEVEL = 200


def main(argv):
    conf = cfg.load()
    short = int(argv[1]) if len(argv) > 1 else min(conf.lamp_short_set())
    hold = float(argv[2]) if len(argv) > 2 else 6.0

    api = Client(conf)
    master = ForeignMaster(conf, api=api)
    master.probe()

    def state():
        st = api.device_full(short).get("state", {})
        return st.get("rgb"), st.get("value_source")

    print("светильник %d, уровень %d, пауза %.0f с на шаг" % (short, LEVEL, hold))
    api.ts(short, {"power": "on", "level": LEVEL})
    time.sleep(2.0)
    try:
        for name, trip in [("ЧУЖОЙ красный", (254, 0, 0)),
                           ("ЧУЖОЙ синий  ", (0, 0, 254))]:
            master.set_rgb(short, *trip)
            time.sleep(hold)
            print("  %s -> rgb=%s source=%s" % ((name,) + state()))
        for name, rgb in [("НАШ   красный", {"r": 254, "g": 0, "b": 0}),
                          ("НАШ   синий  ", {"r": 0, "g": 0, "b": 254})]:
            api.ts(short, {"color_mode": "rgb", "rgb": rgb})
            time.sleep(hold)
            print("  %s -> rgb=%s source=%s" % ((name,) + state()))
    finally:
        api.off(short)
    print("готово; лампа погашена")


if __name__ == "__main__":
    main(sys.argv)

from hil.wait import wait_until


def roles(api, peer_api):
    return api.redundancy.get(), peer_api.redundancy.get()


def settled(api, peer_api):
    a, s = roles(api, peer_api)
    return a["active"] and not s["active"]


def settle(api, peer_api, timeout_s=15.0):
    if settled(api, peer_api):
        return False
    a, s = roles(api, peer_api)
    if s["active"] and not a["active"]:
        peer_api.redundancy.switchover()
    else:
        peer_api.dali_settings.patch({"application_active": False})
        api.dali_settings.patch({"application_active": True})
    assert wait_until(lambda: settled(api, peer_api), timeout_s, 0.5), \
        "the pair could not be put back to primary-active / peer-passive: %s / %s" \
        % roles(api, peer_api)
    return True


def hand_back(api, peer_api):
    if peer_api is None:
        return
    try:
        if settle(api, peer_api):
            print("dut reboot: the peer held the bus while the DUT was down; handed back")
    except Exception as exc:
        print("dut reboot: pair roles NOT restored (%s) — later writes may be refused" % exc)

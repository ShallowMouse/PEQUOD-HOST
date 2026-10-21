#!/usr/bin/env python3
"""A pretend PEQUOD FC on Wi-Fi (UDP 7400), for testing PEQUOD Host without the hardware.

    python3 fake_controller.py                 # answer PEQUOD, print what it sends
    python3 fake_controller.py --script demo   # also press some switches and move the pedal

It speaks the same protocol as Controller/PEQUOD_FC (docs/controller-protocol.md).
"""
import argparse, json, socket, sys, threading, time

PORT = 7400
UNITS = [   # like the real one: strip + 2x2 in the master case, two 2x2 boards, expression pedal
    {"slot": 0, "type": 21, "kind": "footswitch", "uid": "22334455"},
    {"slot": 1, "type": 20, "kind": "footswitch", "uid": "1A2B3C4D"},
    {"slot": 2, "type": 20, "kind": "footswitch", "uid": "6A7B8C9D"},
    {"slot": 3, "type": 20, "kind": "footswitch", "uid": "0E1F2A3B"},
    {"slot": 4, "type": 41, "kind": "pedal_switch", "uid": "5E6F7081"},
    {"slot": 5, "type": 100, "kind": "encoder", "uid": "9A0B1C2D"},
]

def codes(kind):
    return {"footswitch": [0, 1, 2, 3], "pedal_switch": [100, 0], "encoder": [200, 201]}[kind]

class Fake:
    def __init__(self, log):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        self.sock.bind(("0.0.0.0", PORT))
        self.peer = None
        self.last_rx = 0
        self.layer = 0
        self.txid = 0
        self.rx = {}
        self.log = log
        self.watch = set()
        self.cfg = {
            "name": "Longsleeves", "brightness": 200, "offGlow": 10, "layers": 2,
            "layerColors": ["#F44336", "#9C27B0", "#2196F3", "#4CAF50"],
            "midi": {"usb": True, "din": True},
            "wifi": {"enabled": True, "ssid": "Studio", "pass": "secret", "ssid2": "", "pass2": "", "hostname": "pequod-fc"},
            "display": {"contrast": 200}, "units": {},
        }
        names = {"footswitch": "Switches", "pedal_switch": "Pedal", "encoder": "Knob"}
        count = {}
        for u in UNITS:
            count[u["kind"]] = count.get(u["kind"], 0) + 1
            ctrls = {}
            for i, c in enumerate(codes(u["kind"])):
                ctrls[str(c)] = {"color": ["#E0A82E", "#4CAF50", "#2196F3", "#E91E63"][i % 4], "led": "auto",
                                 "layers": [{"mode": "pequod", "num": 20 + l * 8 + i, "ch": 1, "toggle": True} for l in range(4)]}
            entry = {"slot": u["slot"], "name": f"{names[u['kind']]} {count[u['kind']]}", "controls": ctrls}
            if u["kind"] == "pedal_switch": entry["pedal"] = {"min": 0, "max": 4095, "invert": False}
            if u["kind"] == "encoder": entry["encoder"] = {"invert": False}
            self.cfg["units"][u["uid"]] = entry
        self.fw = None

    # framing: "PQL1" 'J' id index count json
    def send(self, obj):
        if not self.peer: return
        data = json.dumps(obj, separators=(",", ":"), ensure_ascii=True).encode()
        chunks = [data[i:i + 1000] for i in range(0, len(data), 1000)] or [b""]
        mid = self.txid & 0x7F; self.txid += 1
        for i, ch in enumerate(chunks):
            self.sock.sendto(b"PQL1J" + bytes([mid, i, len(chunks)]) + ch, self.peer)

    def info(self):
        units = [dict(u, name=self.cfg["units"][u["uid"]]["name"], layer=self.layer) for u in UNITS]
        return {"t": "info", "proto": 2, "fw": "2.0.0-fake", "id": "F00DCAFE0001", "name": self.cfg["name"], "via": "wifi",
                "layers": self.cfg["layers"], "layer": self.layer, "globalLayers": True, "brightness": self.cfg["brightness"],
                "wifi": {"connected": True, "ap": False, "ssid": self.cfg["wifi"]["ssid"], "ssid2": "", "ip": "127.0.0.1", "rssi": -51},
                "ota": {"ok": True, "max": 1966080}, "units": units}

    def public_cfg(self):
        c = json.loads(json.dumps(self.cfg))
        for k, h in (("pass", "hasPass"), ("pass2", "hasPass2")):
            c["wifi"][h] = bool(c["wifi"].get(k)); c["wifi"][k] = None
        return c

    def handle(self, m):
        t = m.get("t")
        if t not in ("ping",): self.log(m)
        if t == "hello": self.send(self.info())
        elif t == "ping": self.send({"t": "pong"})
        elif t == "get_cfg": self.send({"t": "cfg", "cfg": self.public_cfg()})
        elif t == "set_cfg":
            new = m["cfg"]
            for k in ("pass", "pass2"):
                if new["wifi"].get(k) is None: new["wifi"][k] = self.cfg["wifi"].get(k, "")
            new["wifi"].pop("hasPass", None); new["wifi"].pop("hasPass2", None)
            self.cfg = new
            self.send({"t": "cfg_ok"})
        elif t == "layer": self.layer = m["l"]; self.send({"t": "layer", "l": self.layer})
        elif t == "wifiscan":
            self.send({"t": "wifilist", "nets": [{"ssid": "Studio", "rssi": -48, "lock": True}, {"ssid": "Cafe", "rssi": -80, "lock": False}]})
        elif t == "cal":
            (self.watch.add if m.get("on") else self.watch.discard)(m["u"])
        elif t == "rescan": self.send(self.info()); self.send({"t": "cfg", "cfg": self.public_cfg()})
        elif t == "fw_begin": self.fw = 0; self.send({"t": "fw_ack", "n": -1, "ok": True})
        elif t == "fw_data": self.send({"t": "fw_ack", "n": m["n"], "ok": True})
        elif t == "fw_end": self.send({"t": "fw_done", "ok": True})

    def run(self):
        while True:
            data, addr = self.sock.recvfrom(2048)
            if len(data) < 8 or data[:5] != b"PQL1J": continue
            self.peer = addr
            self.last_rx = time.time()
            mid, idx, cnt = data[5], data[6], data[7]
            r = self.rx.setdefault(addr, {"id": -1, "parts": []})
            if idx == 0: r["id"], r["parts"] = mid, []
            if r["id"] != mid or len(r["parts"]) != idx: continue
            r["parts"].append(data[8:])
            if len(r["parts"]) == cnt:
                try: self.handle(json.loads(b"".join(r["parts"])))
                except Exception as e: print("bad message", e, file=sys.stderr)

    # pretend to play
    def press(self, uid, code, down):
        self.send({"t": "press", "u": uid, "c": code, "v": 1 if down else 0})
        mode = self.cfg["units"][uid]["controls"][str(code)]["layers"][self.layer]["mode"]
        if mode == "pequod": self.send({"t": "ev", "u": uid, "c": code, "l": self.layer, "v": 1 if down else 0})

    def tap(self, uid, code):
        self.press(uid, code, True); time.sleep(0.12); self.press(uid, code, False)

    def pedal(self, uid, f):
        self.send({"t": "ev", "u": uid, "c": 100, "l": self.layer, "v": round(f * 127), "f": f})
        if uid in self.watch: self.send({"t": "raw", "u": uid, "raw": int(300 + f * 3400), "v": round(f * 127)})

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--script", default="")
    ap.add_argument("--log", default="")
    a = ap.parse_args()
    out = open(a.log, "w") if a.log else sys.stdout
    def log(m): out.write(json.dumps(m) + "\n"); out.flush()
    f = Fake(log)
    threading.Thread(target=f.run, daemon=True).start()
    if a.script == "demo":
        while f.peer is None: time.sleep(0.2)
        time.sleep(10)
        for _ in range(2):
            f.tap("1A2B3C4D", 1); time.sleep(1.5)
        for i in range(30): f.pedal("5E6F7081", i / 29); time.sleep(0.05)
    while True: time.sleep(1)

if __name__ == "__main__":
    main()

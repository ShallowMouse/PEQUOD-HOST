#include "ControllerLink.h"

namespace
{
    constexpr double pingEveryMs     = 1000.0;
    constexpr double timeoutMs       = 4500.0;
    constexpr double discoverEveryMs = 2000.0;
    constexpr double usbScanEveryMs  = 1500.0;
    constexpr double probeWaitMs     = 1500.0;
    constexpr double probeRetryMs    = 4000.0;
    constexpr int    probeTries      = 3;

    juce::var makeMsg (const char* type)
    {
        auto* o = new juce::DynamicObject();
        o->setProperty ("t", type);
        return juce::var (o);
    }
}

//==============================================================================
ControllerInfo ControllerInfo::fromVar (const juce::var& v)
{
    ControllerInfo i;
    i.id = v["id"].toString();
    i.name = v["name"].toString();
    i.fw = v["fw"].toString();
    i.via = v["via"].toString();
    i.proto = (int) v["proto"];
    i.layers = juce::jlimit (1, 4, (int) v.getProperty ("layers", 1));
    i.layer = (int) v["layer"];
    i.brightness = (int) v.getProperty ("brightness", 255);
    i.globalLayers = (bool) v["globalLayers"];
    auto w = v["wifi"];
    i.wifiConnected = (bool) w["connected"];
    i.wifiAp = (bool) w["ap"];
    i.ssid = w["ssid"].toString();
    i.ssid2 = w["ssid2"].toString();
    i.ip = w["ip"].toString();
    auto o = v["ota"];
    i.otaOk = (bool) o["ok"];
    i.otaMax = (int64_t) (double) o["max"];
    if (auto* arr = v["units"].getArray())
        for (auto& u : *arr)
        {
            ControllerUnit cu;
            cu.slot = (int) u["slot"];
            cu.type = (int) u["type"];
            cu.layer = (int) u["layer"];
            cu.kind = u["kind"].toString();
            cu.name = u["name"].toString();
            cu.uid = u["uid"].toString();
            i.units.push_back (cu);
        }
    std::sort (i.units.begin(), i.units.end(), [] (auto& a, auto& b) { return a.slot < b.slot; });
    return i;
}

//==============================================================================
juce::String LinkFraming::toAsciiJson (const juce::var& v)
{
    auto json = juce::JSON::toString (v, true);
    juce::String out;
    out.preallocateBytes ((size_t) json.length() + 16);
    for (auto p = json.getCharPointer(); ! p.isEmpty(); ++p)
    {
        auto c = *p;
        if (c < 0x80) out += (juce::juce_wchar) c;
        else if (c <= 0xFFFF) out += "\\u" + juce::String::toHexString ((int) c).paddedLeft ('0', 4);
        else out += "?";
    }
    return out;
}

std::vector<juce::MemoryBlock> LinkFraming::encode (const juce::String& json, uint8_t msgId, bool forSysEx)
{
    const auto bytes = json.toStdString();
    const int maxPayload = forSysEx ? sysExPayload : udpPayload;
    const int len = (int) bytes.size();
    const int count = juce::jmax (1, (len + maxPayload - 1) / maxPayload);
    std::vector<juce::MemoryBlock> frames;
    if (count > 127) return frames;

    for (int i = 0; i < count; ++i)
    {
        juce::MemoryOutputStream o;
        if (forSysEx) { o.writeByte ((char) 0xF0); o.writeByte (0x7D); o.writeByte ('P'); o.writeByte ('Q'); }
        else          { o.write ("PQL1", 4); }
        o.writeByte ('J');
        o.writeByte ((char) (msgId & 0x7F));
        o.writeByte ((char) i);
        o.writeByte ((char) count);
        const int start = i * maxPayload, end = juce::jmin (len, start + maxPayload);
        for (int k = start; k < end; ++k) o.writeByte ((char) (bytes[(size_t) k] & 0x7F));
        if (forSysEx) o.writeByte ((char) 0xF7);
        frames.push_back (o.getMemoryBlock());
    }
    return frames;
}

bool LinkFraming::Assembler::feed (const uint8_t* frame, int len, juce::String& out)
{
    if (len < 4 || frame[0] != 'J') return false;
    const int fid = frame[1], index = frame[2], fcount = frame[3];
    if (index == 0) { id = fid; next = 0; count = fcount; text.reset(); }
    if (id != fid || index != next || fcount != count) { id = -1; return false; }
    text.write (frame + 4, (size_t) (len - 4));
    if (++next < count) return false;
    out = text.toString();
    text.reset();
    id = -1;
    return true;
}

//==============================================================================
ControllerLink::ControllerLink (juce::AudioDeviceManager& dm)
    : Thread ("PEQUOD Link UDP"), deviceManager (dm)
{
    socket = std::make_unique<juce::DatagramSocket> (true);
    if (! socket->bindToPort (0))
        socket.reset();

    deviceManager.addMidiInputDeviceCallback ({}, this);
    if (socket != nullptr) startThread();
    startTimer (100);
}

ControllerLink::~ControllerLink()
{
    stopTimer();
    if (isConnected()) send (makeMsg ("bye"));
    signalThreadShouldExit();
    if (socket != nullptr) socket->shutdown();
    stopThread (1000);
    ownInputs.clear();
    deviceManager.removeMidiInputDeviceCallback ({}, this);
}

juce::String ControllerLink::describeConnection() const
{
    switch (via)
    {
        case Via::usb:  return "USB";
        case Via::wifi: return "Wi-Fi " + wifiIp;
        case Via::none: default: return "Not connected";
    }
}

juce::StringArray ControllerLink::describeUsbPorts() const
{
    juce::StringArray out;
    for (auto& d : juce::MidiInput::getAvailableDevices())
    {
        auto p = std::find_if (probes.begin(), probes.end(), [&] (const Probe& x) { return x.inputId == d.identifier; });
        juce::String state = p == probes.end() ? "input only (no matching output)"
                           : ! p->failed ? "asking..."
                           : p->tries < probeTries ? "no answer yet"
                           : "no answer - not a PEQUOD FC, or in use by another program";
        out.add (d.name + " - " + state);
    }
    return out;
}

//==============================================================================
bool ControllerLink::send (const juce::var& message)
{
    if (via == Via::usb)  return sendTo (message, Via::usb, {});
    if (via == Via::wifi) return sendTo (message, Via::wifi, wifiIp);
    return false;
}

bool ControllerLink::sendTo (const juce::var& message, Via to, const juce::String& target)
{
    auto json = LinkFraming::toAsciiJson (message);
    if (to == Via::usb)
    {
        const juce::ScopedLock sl (sendLock);
        return usbOut != nullptr && sendUsbRaw (*usbOut, json);
    }
    return sendUdpRaw (target, json);
}

bool ControllerLink::sendUsbRaw (juce::MidiOutput& out, const juce::String& json)
{
    for (auto& f : LinkFraming::encode (json, (uint8_t) (++msgId & 0x7F), true))
        out.sendMessageNow (juce::MidiMessage (f.getData(), (int) f.getSize()));
    return true;
}

bool ControllerLink::sendUdpRaw (const juce::String& ip, const juce::String& json)
{
    if (socket == nullptr || ip.isEmpty()) return false;
    const juce::ScopedLock sl (sendLock);
    for (auto& f : LinkFraming::encode (json, (uint8_t) (++msgId & 0x7F), false))
        if (socket->write (ip, LinkFraming::udpPort, f.getData(), (int) f.getSize()) < 0)
            return false;
    return true;
}

//==============================================================================
void ControllerLink::timerCallback()
{
    const double now = juce::Time::getMillisecondCounterHiRes();

    if (isConnected() && now - lastRx > timeoutMs)
        disconnect();

    if (isConnected() && now - lastPing > pingEveryMs)
    {
        lastPing = now;
        send (makeMsg ("ping"));
    }

    if (via != Via::usb && now - lastUsbScan > usbScanEveryMs)
    {
        lastUsbScan = now;
        scanUsb();
    }

    // Probes that never answered are not controllers.
    for (auto& p : probes)
        if (! p.failed && p.sentAt > 0 && now - p.sentAt > probeWaitMs)
        {
            p.failed = true;
            // Not a controller: let go of the port so other software (and AUDIO > MIDI inputs) can use it.
            if (p.tries >= probeTries)
                ownInputs.erase (p.inputId);
        }

    if (via == Via::none && now - lastDiscover > discoverEveryMs)
    {
        lastDiscover = now;
        discoverWifi();
    }
}

void ControllerLink::scanUsb()
{
    auto ins = juce::MidiInput::getAvailableDevices();
    auto outs = juce::MidiOutput::getAvailableDevices();
    const double now = juce::Time::getMillisecondCounterHiRes();

    juce::StringArray signature;
    for (auto& d : ins)  signature.add ("i:" + d.identifier);
    for (auto& d : outs) signature.add ("o:" + d.identifier);
    if (signature != lastDeviceSignature)
    {
        lastDeviceSignature = signature;
        probes.clear();   // something was plugged in or out: look again

        for (auto it = ownInputs.begin(); it != ownInputs.end();)
        {
            const bool present = std::any_of (ins.begin(), ins.end(), [&] (auto& d) { return d.identifier == it->first; });
            if (present && it->first == usbInputId) { ++it; continue; }
            it = ownInputs.erase (it);
        }
    }

    for (auto& in : ins)
    {
        for (auto& out : outs)
        {
            if (out.name != in.name) continue;   // a device's input and output share its name

            auto known = std::find_if (probes.begin(), probes.end(), [&] (const Probe& p) { return p.inputId == in.identifier; });
            if (known != probes.end())
            {
                // No answer yet (the controller may still have been starting up): ask again a few times.
                if (known->failed && known->tries < probeTries && now - known->sentAt > probeRetryMs)
                {
                    known->failed = false;
                    known->tries++;
                    known->sentAt = now;
                    sendHello (known->outputId);
                }
                break;
            }

            // Listen to the port even if it isn't switched on under AUDIO > MIDI inputs.
            if (! deviceManager.isMidiInputDeviceEnabled (in.identifier) && ownInputs.count (in.identifier) == 0)
                if (auto mi = juce::MidiInput::openDevice (in.identifier, this))
                {
                    mi->start();
                    ownInputs[in.identifier] = std::move (mi);
                }

            Probe p { in.identifier, out.identifier, now, false, 1, in.name };
            probes.push_back (p);
            sendHello (out.identifier);
            break;
        }
    }
}

void ControllerLink::sendHello (const juce::String& outputId)
{
    if (auto o = juce::MidiOutput::openDevice (outputId))
    {
        auto hello = makeMsg ("hello");
        hello.getDynamicObject()->setProperty ("app", "PEQUOD Host");
        hello.getDynamicObject()->setProperty ("proto", 2);
        sendUsbRaw (*o, LinkFraming::toAsciiJson (hello));
    }
}

void ControllerLink::discoverWifi()
{
    if (socket == nullptr) return;
    auto hello = makeMsg ("hello");
    hello.getDynamicObject()->setProperty ("app", "PEQUOD Host");
    hello.getDynamicObject()->setProperty ("proto", 2);
    auto json = LinkFraming::toAsciiJson (hello);
    sendUdpRaw ("255.255.255.255", json);
    if (manualIp.isNotEmpty())
        sendUdpRaw (manualIp, json);
}

//==============================================================================
void ControllerLink::handleIncomingMidiMessage (juce::MidiInput* source, const juce::MidiMessage& m)
{
    if (! m.isSysEx() || source == nullptr) return;
    const auto* d = m.getSysExData();         // without F0 / F7
    const int n = m.getSysExDataSize();
    if (n < 7 || d[0] != 0x7D || d[1] != 'P' || d[2] != 'Q') return;

    juce::MemoryBlock frame (d + 3, (size_t) (n - 3));
    auto id = source->getIdentifier();
    juce::MessageManager::callAsync ([this, frame, id]
    {
        juce::String json;
        if (usbAssemblers[id].feed ((const uint8_t*) frame.getData(), (int) frame.getSize(), json))
            handleMessage (json, Via::usb, id);
    });
}

void ControllerLink::run()
{
    juce::HeapBlock<uint8_t> buf (2048);
    while (! threadShouldExit())
    {
        if (socket->waitUntilReady (true, 200) != 1) continue;
        juce::String senderIp;
        int senderPort = 0;
        const int n = socket->read (buf, 2048, false, senderIp, senderPort);
        if (n < 8 || memcmp (buf, "PQL1", 4) != 0) continue;

        juce::MemoryBlock frame (buf + 4, (size_t) (n - 4));
        juce::MessageManager::callAsync ([this, frame, senderIp]
        {
            juce::String json;
            if (udpAssembler.feed ((const uint8_t*) frame.getData(), (int) frame.getSize(), json))
                handleMessage (json, Via::wifi, senderIp);
        });
    }
}

//==============================================================================
void ControllerLink::handleMessage (const juce::String& json, Via from, const juce::String& source)
{
    auto msg = juce::JSON::parse (json);
    if (! msg.isObject()) return;
    const auto type = msg["t"].toString();

    if (type == "info")
    {
        auto newInfo = ControllerInfo::fromVar (msg);

        if (! isConnected())
        {
            connectVia (from, source, newInfo);
            return;
        }

        // USB beats Wi-Fi for the same controller.
        if (via == Via::wifi && from == Via::usb && newInfo.id == info.id)
        {
            connectVia (from, source, newInfo);
            return;
        }

        const bool fromCurrent = (from == via) && (from == Via::wifi ? source == wifiIp : source == usbInputId);
        if (! fromCurrent) return;     // a second controller, or the other route: ignore

        info = newInfo;
        lastRx = juce::Time::getMillisecondCounterHiRes();
        listeners.call ([] (Listener& l) { l.controllerInfoChanged(); });
        return;
    }

    const bool fromCurrent = isConnected() && from == via && (from == Via::wifi ? source == wifiIp : source == usbInputId);
    if (! fromCurrent) return;

    lastRx = juce::Time::getMillisecondCounterHiRes();
    if (type == "pong") return;
    listeners.call ([&] (Listener& l) { l.controllerMessage (type, msg); });
}

void ControllerLink::connectVia (Via v, const juce::String& source, const ControllerInfo& newInfo)
{
    const bool wasConnected = isConnected();

    if (v == Via::usb)
    {
        auto probe = std::find_if (probes.begin(), probes.end(), [&] (const Probe& p) { return p.inputId == source; });
        if (probe == probes.end()) return;
        auto out = juce::MidiOutput::openDevice (probe->outputId);
        if (out == nullptr) return;
        const juce::ScopedLock sl (sendLock);
        usbOut = std::move (out);
        usbInputId = source;
    }
    else
    {
        wifiIp = source;
    }

    via = v;
    info = newInfo;
    lastRx = lastPing = juce::Time::getMillisecondCounterHiRes();

    if (wasConnected) listeners.call ([] (Listener& l) { l.controllerInfoChanged(); });
    else              listeners.call ([] (Listener& l) { l.controllerConnected(); });
}

void ControllerLink::disconnect()
{
    {
        const juce::ScopedLock sl (sendLock);
        usbOut.reset();
    }
    via = Via::none;
    usbInputId.clear();
    wifiIp.clear();
    probes.clear();
    listeners.call ([] (Listener& l) { l.controllerDisconnected(); });
}

#include "ControllerManager.h"

#if PEQUOD_HAS_FIRMWARE
 #include <FirmwareData.h>
#endif

namespace
{
    juce::var obj() { return juce::var (new juce::DynamicObject()); }

    juce::var msg (const char* type)
    {
        auto v = obj();
        v.getDynamicObject()->setProperty ("t", type);
        return v;
    }

    void set (juce::var& v, const juce::Identifier& k, const juce::var& value) { v.getDynamicObject()->setProperty (k, value); }

    juce::String key (const juce::String& uid, int control) { return uid + ":" + juce::String (control); }

    juce::String hex (juce::Colour c) { return "#" + c.toDisplayString (false); }

    /** The controller's screen font is plain ASCII. */
    juce::String asciiFold (const juce::String& s)
    {
        static const char* from = "\xC3\xA0\xC3\xA1\xC3\xA2\xC3\xA3\xC3\xA4\xC3\xA5\xC3\xA8\xC3\xA9\xC3\xAA\xC3\xAB\xC3\xAC\xC3\xAD\xC3\xAE\xC3\xAF\xC3\xB2\xC3\xB3\xC3\xB4\xC3\xB5\xC3\xB6\xC3\xB9\xC3\xBA\xC3\xBB\xC3\xBC\xC3\xB1\xC3\xA7";
        static const char* to   = "aaaaaaeeeeiiiiooooouuuunc";
        auto src = juce::String::fromUTF8 (from);
        juce::String out;
        for (auto p = s.getCharPointer(); ! p.isEmpty(); ++p)
        {
            auto c = juce::CharacterFunctions::toLowerCase (*p);
            const bool upper = c != *p;
            if (*p < 0x80) { out += *p; continue; }
            const int i = src.indexOfChar (c);
            if (i >= 0) { juce::juce_wchar r = (juce::juce_wchar) to[i]; out += upper ? juce::CharacterFunctions::toUpperCase (r) : r; }
            else if (*p == 0x2013 || *p == 0x2014) out += "-";
            else if (*p == 0x2019 || *p == 0x2018) out += "'";
            else out += "?";
        }
        return out;
    }

    bool isMomentaryTarget (Mapping::Target t)
    {
        return t == Mapping::Target::presetNext || t == Mapping::Target::presetPrev
            || t == Mapping::Target::bankNext   || t == Mapping::Target::bankPrev;
    }
}

//==============================================================================
/** Sends firmware in 768-byte pieces, waiting for the controller to confirm each one. */
class FirmwareUpdater : public juce::Thread
{
public:
    FirmwareUpdater (ControllerManager& m, juce::MemoryBlock data)
        : Thread ("PEQUOD firmware update"), manager (m), bin (std::move (data)) {}

    ~FirmwareUpdater() override { signalThreadShouldExit(); ackEvent.signal(); stopThread (4000); }

    void onAck (int n, bool ok, const juce::String& err)
    {
        const juce::ScopedLock sl (lock);
        ackN = n; ackOk = ok; ackErr = err; ackFresh = true;
        ackEvent.signal();
    }

    void onDone (bool ok, const juce::String& err)
    {
        const juce::ScopedLock sl (lock);
        doneOk = ok; doneErr = err; doneFresh = true;
        ackEvent.signal();
    }

    std::atomic<float> progress { 0.0f };
    std::atomic<bool> finished { false }, succeeded { false };

    juce::String getStatus() const { const juce::ScopedLock sl (lock); return status; }

private:
    ControllerManager& manager;
    juce::MemoryBlock bin;
    juce::CriticalSection lock;
    juce::WaitableEvent ackEvent;
    int ackN = -99; bool ackOk = false, ackFresh = false; juce::String ackErr;
    bool doneOk = false, doneFresh = false; juce::String doneErr;
    juce::String status;

    void setStatus (const juce::String& s) { const juce::ScopedLock sl (lock); status = s; }

    bool waitAck (int n, int timeoutMs, juce::String& err)
    {
        const auto until = juce::Time::getMillisecondCounter() + (juce::uint32) timeoutMs;
        while (! threadShouldExit())
        {
            {
                const juce::ScopedLock sl (lock);
                if (ackFresh && ackN == n) { ackFresh = false; err = ackErr; return ackOk; }
                ackFresh = false;
            }
            const auto now = juce::Time::getMillisecondCounter();
            if (now >= until) { err = "no answer"; return false; }
            ackEvent.wait ((int) (until - now));
        }
        err = "cancelled";
        return false;
    }

    bool sendWithRetry (const juce::var& m, int n, int timeoutMs, juce::String& err)
    {
        for (int attempt = 0; attempt < 5 && ! threadShouldExit(); ++attempt)
        {
            manager.link.send (m);
            if (waitAck (n, timeoutMs, err)) return true;
            if (err != "no answer") return false;      // the controller refused: don't retry
        }
        return false;
    }

    void run() override
    {
        auto fail = [this] (const juce::String& why) { setStatus ("Update failed: " + why); finished = true; };

        setStatus ("Starting update...");
        auto begin = msg ("fw_begin");
        set (begin, "size", (int) bin.getSize());
        set (begin, "md5", juce::MD5 (bin).toHexString());
        juce::String err;
        if (! sendWithRetry (begin, -1, 5000, err)) return fail (err);

        constexpr int chunk = 768;
        const int total = (int) ((bin.getSize() + chunk - 1) / chunk);
        for (int n = 0; n < total && ! threadShouldExit(); ++n)
        {
            const auto offset = (size_t) n * chunk;
            const auto len = juce::jmin ((size_t) chunk, bin.getSize() - offset);
            auto data = msg ("fw_data");
            set (data, "n", n);
            set (data, "d", juce::Base64::toBase64 (static_cast<const char*> (bin.getData()) + offset, len));
            if (! sendWithRetry (data, n, 4000, err)) return fail (err);
            progress = (float) (n + 1) / (float) total;
            setStatus ("Sending firmware... " + juce::String (juce::roundToInt (progress * 100.0f)) + "%");
        }
        if (threadShouldExit()) { manager.link.send (msg ("fw_abort")); return fail ("cancelled"); }

        setStatus ("Checking and installing...");
        { const juce::ScopedLock sl (lock); doneFresh = false; }
        manager.link.send (msg ("fw_end"));
        const auto until = juce::Time::getMillisecondCounter() + 20000;
        while (! threadShouldExit() && juce::Time::getMillisecondCounter() < until)
        {
            {
                const juce::ScopedLock sl (lock);
                if (doneFresh)
                {
                    if (! doneOk) { status = "Update failed: " + doneErr; finished = true; return; }
                    status = "Updated. The controller is restarting...";
                    succeeded = true; finished = true;
                    return;
                }
            }
            ackEvent.wait (200);
        }
        fail ("the controller didn't confirm the update");
    }
};

//==============================================================================
ControllerManager::ControllerManager (HostEngine& e, ControlSurface& c, juce::PropertiesFile& p)
    : engine (e), control (c), props (p), link (e.deviceManager)
{
    link.setManualAddress (props.getValue ("controllerIp"));
    link.addListener (this);

    control.controllerSourceName = [this] (const Mapping& m) { return describeSource (m); };
    control.onControllerMappingAdded = [this] (const Mapping& m) { claim (m.ctrlUnit, m.ctrlControl, m.ctrlLayer); };

    startTimerHz (30);
}

ControllerManager::~ControllerManager()
{
    stopTimer();
    updater.reset();
    control.controllerSourceName = nullptr;
    control.onControllerMappingAdded = nullptr;
    link.removeListener (this);
}

//==============================================================================
juce::String ControllerManager::controlName (const ControllerUnit& u, int c)
{
    auto unit = u.name + " " + juce::String (u.slot + 1);
    if (c == Mapping::ctrlPedal)        return unit;
    if (c == Mapping::ctrlEncoderTurn)  return unit + " turn";
    if (c == Mapping::ctrlEncoderPress) return unit + " press";
    if (u.kind == "pedal_switch")       return unit + " switch";
    return unit + " / " + juce::String (c + 1);
}

juce::String ControllerManager::describeSource (const Mapping& m) const
{
    if (auto* u = getInfo().findUnit (m.ctrlUnit))
    {
        auto s = controlName (*u, m.ctrlControl);
        if (getInfo().layers > 1) s << "  L" << (m.ctrlLayer + 1);
        return s;
    }
    return "Controller (not connected)";
}

juce::String ControllerManager::shortLabel (const Mapping& m) const
{
    switch (m.target)
    {
        case Mapping::Target::bypass:
            if (auto s = engine.findSlot (m.slotUid)) return s->description.name.toUpperCase();
            return "MODULE";
        case Mapping::Target::param:
            if (auto s = engine.findSlot (m.slotUid))
                if (auto* inst = s->instance())
                    if (juce::isPositiveAndBelow (m.paramIndex, inst->getParameters().size()))
                        return inst->getParameters()[m.paramIndex]->getName (14).toUpperCase();
            return "PARAMETER";
        case Mapping::Target::splitMix:   return "SPLIT " + juce::String (engine.getSplitNumber (m.slotUid)) + " A/B";
        case Mapping::Target::inputGain:  return "INPUT";
        case Mapping::Target::outputGain: return "VOLUME";
        case Mapping::Target::presetNext: return "PRESET +";
        case Mapping::Target::presetPrev: return "PRESET -";
        case Mapping::Target::bankNext:   return "BANK +";
        case Mapping::Target::bankPrev:   return "BANK -";
        case Mapping::Target::mute:       return "MUTE";
        case Mapping::Target::tuner:      return "TUNER";
    }
    return {};
}

juce::Colour ControllerManager::defaultColour (Mapping::Target t)
{
    switch (t)
    {
        case Mapping::Target::splitMix:   return juce::Colour (0xff3d9be9);
        case Mapping::Target::presetNext: case Mapping::Target::presetPrev:
        case Mapping::Target::bankNext:   case Mapping::Target::bankPrev: return juce::Colour (0xffeae4d6);
        case Mapping::Target::mute:       return juce::Colour (0xffe8402e);
        case Mapping::Target::tuner:      return juce::Colour (0xff35c96a);
        default:                          return juce::Colour (0xffe0a82e);   // PEQUOD mustard
    }
}

juce::Colour ControllerManager::colourFor (const Mapping& m) const
{
    return m.ledColour != 0 ? juce::Colour (m.ledColour).withAlpha (1.0f) : defaultColour (m.target);
}

bool ControllerManager::mappingIsOn (const Mapping& m) const
{
    switch (m.target)
    {
        case Mapping::Target::bypass:   if (auto s = engine.findSlot (m.slotUid)) return ! s->bypassed.load(); return false;
        case Mapping::Target::param:
            if (auto s = engine.findSlot (m.slotUid))
                if (auto* inst = s->instance())
                    if (juce::isPositiveAndBelow (m.paramIndex, inst->getParameters().size()))
                        return inst->getParameters()[m.paramIndex]->getValue() >= 0.5f;
            return false;
        case Mapping::Target::splitMix:
            for (auto& item : engine.getItems()) if (item.uid == m.slotUid) return item.mix >= 0.5f;
            return false;
        case Mapping::Target::mute:     return control.isMuted();
        case Mapping::Target::tuner:    return control.isTunerMode();
        default:                        return false;
    }
}

juce::String ControllerManager::flashFor (const Mapping& m) const
{
    switch (m.target)
    {
        case Mapping::Target::bypass:   return shortLabel (m) + (mappingIsOn (m) ? " ON" : " OFF");
        case Mapping::Target::param:    return shortLabel (m) + (mappingIsOn (m) ? " ON" : " OFF");
        case Mapping::Target::splitMix: return shortLabel (m).upToLastOccurrenceOf (" ", false, false) + (mappingIsOn (m) ? " > B" : " > A");
        case Mapping::Target::mute:     return control.isMuted() ? "MUTED" : "UNMUTED";
        default:                        return {};
    }
}

//==============================================================================
juce::String ControllerManager::uidForSlot (int slot) const
{
    if (auto* u = getInfo().findSlot (slot)) return u->uid;
    return {};
}

bool ControllerManager::isPressed (const juce::String& uid, int c) const
{
    auto it = pressedNow.find (key (uid, c));
    return it != pressedNow.end() && it->second;
}

float ControllerManager::pedalValue (const juce::String& uid) const
{
    auto it = pedals.find (uid);
    return it != pedals.end() ? it->second : 0.0f;
}

juce::Colour ControllerManager::ledColourShown (const juce::String& uid, int c) const
{
    auto it = ledShown.find (key (uid, c));
    return it != ledShown.end() ? it->second : juce::Colours::transparentBlack;
}

int ControllerManager::pedalRaw (int slot) const
{
    auto it = rawPedals.find (slot);
    return it != rawPedals.end() ? it->second : -1;
}

juce::var ControllerManager::getUnitConfig (int slot) const
{
    auto it = cfgCache.find (slot);
    return it != cfgCache.end() ? it->second : juce::var();
}

//== Ownership ===================================================================
juce::String ControllerManager::controlType (int slot, const ControllerUnit& u, int c, int layer) const
{
    auto cfg = getUnitConfig (slot);
    if (! cfg.isObject()) return {};
    if (c == Mapping::ctrlEncoderTurn)  return cfg["encoder"]["rotationMode"].toString();
    if (c == Mapping::ctrlEncoderPress) return cfg["encoder"]["buttonMode"].toString();
    const int L = (c == Mapping::ctrlPedal) ? 0 : juce::jlimit (0, 3, layer);
    const int b = (c == Mapping::ctrlPedal) ? 0 : c;
    juce::ignoreUnused (u);
    return cfg["layers"][L]["buttons"][b]["type"].toString();
}

juce::var ControllerManager::typePatch (const ControllerUnit& u, int c, int layer, const juce::String& type) const
{
    juce::ignoreUnused (u);
    auto patch = obj();
    if (c == Mapping::ctrlEncoderTurn || c == Mapping::ctrlEncoderPress)
    {
        auto e = obj();
        set (e, c == Mapping::ctrlEncoderTurn ? "rotationMode" : "buttonMode", type);
        set (patch, "encoder", e);
        return patch;
    }
    const int L = (c == Mapping::ctrlPedal) ? 0 : juce::jlimit (0, 3, layer);
    const int b = (c == Mapping::ctrlPedal) ? 0 : c;
    juce::Array<juce::var> buttons;
    for (int i = 0; i < b; ++i) buttons.add (juce::var());
    auto btn = obj(); set (btn, "type", type);
    buttons.add (btn);
    auto layerObj = obj(); set (layerObj, "buttons", buttons);
    juce::Array<juce::var> layers;
    for (int i = 0; i < L; ++i) layers.add (juce::var());
    layers.add (layerObj);
    set (patch, "layers", layers);
    return patch;
}

bool ControllerManager::isClaimed (const ControllerUnit& u, int c, int layer) const
{
    return controlType (u.slot, u, c, layer) == "pequod";
}

juce::var ControllerManager::prevTypes() const
{
    auto v = juce::JSON::parse (props.getValue ("controllerPrevTypes"));
    return v.isObject() ? v : obj();
}

void ControllerManager::setPrevType (const juce::String& k, const juce::String& type)
{
    auto v = prevTypes();
    if (type.isEmpty()) v.getDynamicObject()->removeProperty (k);
    else set (v, k, type);
    props.setValue ("controllerPrevTypes", juce::JSON::toString (v, true));
}

void ControllerManager::claim (const juce::String& uid, int c, int layer)
{
    auto* u = getInfo().findUnit (uid);
    if (u == nullptr) return;
    if (! getUnitConfig (u->slot).isObject())
    {
        pendingClaims.emplace_back (uid, c, layer);   // once its settings arrive
        return;
    }
    const auto current = controlType (u->slot, *u, c, layer);
    if (current == "pequod") return;

    setPrevType (getInfo().id + ":" + uid + ":" + juce::String (c) + ":" + juce::String (layer), current);
    setUnitConfig (u->slot, typePatch (*u, c, layer, "pequod"));
}

void ControllerManager::release (const ControllerUnit& u, int c, int layer)
{
    const auto k = getInfo().id + ":" + u.uid + ":" + juce::String (c) + ":" + juce::String (layer);
    auto prev = prevTypes()[juce::Identifier (k)].toString();
    if (prev.isEmpty() || prev == "pequod")
        prev = (c == Mapping::ctrlPedal) ? "cc" : (c == Mapping::ctrlEncoderTurn) ? "layer_global"
             : (c == Mapping::ctrlEncoderPress) ? "layer_toggle" : "note";
    control.clearControllerMapping (u.uid, c, layer);
    setUnitConfig (u.slot, typePatch (u, c, layer, prev));
    setPrevType (k, {});
    if (onStatus) onStatus (controlName (u, c) + " uses its own MIDI settings again");
}

void ControllerManager::setLedColour (const juce::String& uid, int c, int layer, juce::Colour col)
{
    if (auto* m = control.findControllerMapping (uid, c, layer))
    {
        Mapping copy = *m;
        copy.ledColour = col.getARGB();
        control.setControllerMapping (copy);
    }
}

void ControllerManager::identify (const ControllerUnit& u, int c)
{
    auto m = msg ("identify");
    set (m, "s", u.slot);
    set (m, "b", u.kind == "pedal_switch" ? 1 : c);
    link.send (m);
}

void ControllerManager::selectLayer (int layer)
{
    auto m = msg ("layer");
    set (m, "l", layer);
    link.send (m);
}

//== Device settings ============================================================
void ControllerManager::setSettings (const juce::var& patch)
{
    auto m = patch.isObject() ? juce::JSON::parse (juce::JSON::toString (patch)) : obj();
    set (m, "t", "set_settings");
    link.send (m);
}

void ControllerManager::setUnitConfig (int slot, const juce::var& patch)
{
    auto m = msg ("set_cfg");
    set (m, "slot", slot);
    set (m, "cfg", patch);
    link.send (m);
}

void ControllerManager::rescanUnits() { link.send (msg ("rescan")); }

void ControllerManager::watchPedal (int slot, bool on)
{
    auto m = msg ("cal");
    set (m, "s", slot);
    set (m, "on", on);
    link.send (m);
}

void ControllerManager::requestConfigs()
{
    for (auto& u : getInfo().units)
    {
        auto m = msg ("get_cfg");
        set (m, "slot", u.slot);
        link.send (m);
    }
    link.send (msg ("get_settings"));
}

//== Link events =================================================================
void ControllerManager::controllerConnected()
{
    cfgCache.clear();
    lastLedSent.clear();
    lastScreen.clear();
    backupPending = true;
    requestConfigs();
    if (onStatus) onStatus ("Controller connected over " + link.describeConnection());
    sendChangeMessage();
}

void ControllerManager::controllerDisconnected()
{
    cfgCache.clear();
    pressedNow.clear();
    ledShown.clear();
    if (onStatus && ! isUpdating()) onStatus ("Controller disconnected");
    sendChangeMessage();
}

void ControllerManager::controllerInfoChanged()
{
    requestConfigs();
    sendChangeMessage();
}

void ControllerManager::onPress (const ControllerUnit& u, int c, int layer, float value, int delta)
{
    const bool hit = control.handleControllerEvent (u.uid, c, layer, value, delta);
    if (! hit || delta != 0 || value < 0.5f || c == Mapping::ctrlPedal) return;
    if (auto* m = control.findControllerMapping (u.uid, c, layer))
    {
        auto f = flashFor (*m);
        if (f.isNotEmpty()) flash (f);
    }
}

void ControllerManager::flash (const juce::String& text)
{
    flashText = text;
    flashUntil = juce::Time::getMillisecondCounterHiRes() + 1200.0;
    updateScreen (true);
}

void ControllerManager::controllerMessage (const juce::String& type, const juce::var& m)
{
    if (type == "ev")
    {
        const int slot = (int) m["s"];
        auto* u = getInfo().findSlot (slot);
        if (u == nullptr) return;
        const auto k = m["k"].toString();
        const int layer = (int) m["l"];

        if (k == "btn")
        {
            const int c = u->kind == "pedal_switch" ? 1 : (int) m["b"];
            const bool v = (int) m["v"] != 0;
            pressedNow[key (u->uid, c)] = v;
            if (v) pressedUntil[key (u->uid, c)] = juce::Time::getMillisecondCounterHiRes() + 160.0;
            onPress (*u, c, layer, v ? 1.0f : 0.0f, 0);
        }
        else if (k == "pedal")
        {
            const float v = (float) (int) m["v"] / 127.0f;
            pedals[u->uid] = v;
            onPress (*u, Mapping::ctrlPedal, layer, v, 0);
        }
        else if (k == "enc")
        {
            onPress (*u, Mapping::ctrlEncoderTurn, layer, 0.5f, (int) m["d"]);
        }
        else if (k == "encbtn")
        {
            const bool v = (int) m["v"] != 0;
            pressedNow[key (u->uid, Mapping::ctrlEncoderPress)] = v;
            onPress (*u, Mapping::ctrlEncoderPress, layer, v ? 1.0f : 0.0f, 0);
        }
        sendChangeMessage();
        return;
    }

    if (type == "layer")
    {
        // Keep the shown page in step with the hardware (shift, encoder, web page).
        auto info = getInfo();
        juce::ignoreUnused (info);
        link.send (msg ("hello"));     // refreshes units + layers via "info"
        return;
    }

    if (type == "cfg")
    {
        const int slot = (int) m["slot"];
        cfgCache[slot] = m["cfg"];

        // Claims that were waiting for this unit's settings.
        auto waiting = pendingClaims;
        pendingClaims.clear();
        for (auto& [uid, c, layer] : waiting) claim (uid, c, layer);

        if (backupPending && cfgCache.size() >= getInfo().units.size())
        {
            backupPending = false;
            saveBackup();
        }
        sendChangeMessage();
        return;
    }

    if (type == "settings") { settingsCache = m; sendChangeMessage(); return; }
    if (type == "raw")      { rawPedals[(int) m["s"]] = (int) m["raw"]; sendChangeMessage(); return; }
    if (type == "fw_ack")   { if (updater) updater->onAck ((int) m["n"], (bool) m["ok"], m["err"].toString()); return; }
    if (type == "fw_done")  { if (updater) updater->onDone ((bool) m["ok"], m["err"].toString()); return; }
    if (type == "err")      { if (onStatus) onStatus ("Controller: " + m["msg"].toString()); return; }
}

//== Feedback ====================================================================
void ControllerManager::timerCallback()
{
    if (! isConnected() || isUpdating()) return;
    updateLeds (false);
    updateScreen (false);
}

void ControllerManager::updateLeds (bool force)
{
    const double now = juce::Time::getMillisecondCounterHiRes();
    juce::Array<juce::var> changes;
    bool uiChanged = false;

    for (auto& u : getInfo().units)
    {
        juce::Array<int> controls;
        if (u.hasButtons()) controls = { 0, 1, 2, 3 };
        else if (u.kind == "pedal_switch") controls = { 1 };

        for (int c : controls)
        {
            const int ledIndex = c;
            const bool claimed = isClaimed (u, c, u.layer);
            juce::String want;
            juce::Colour shown = juce::Colours::transparentBlack;

            if (claimed)
            {
                if (auto* m = control.findControllerMapping (u.uid, c, u.layer))
                {
                    const auto col = colourFor (*m);
                    bool on = mappingIsOn (*m);
                    if (isMomentaryTarget (m->target))
                    {
                        auto it = pressedUntil.find (key (u.uid, c));
                        on = it != pressedUntil.end() && it->second > now;
                    }
                    shown = on ? col : col.withMultipliedBrightness (0.14f);
                    want = hex (shown);
                }
                else
                {
                    want = "#000000";     // PEQUOD's switch, nothing on it in this preset
                    shown = juce::Colours::black;
                }
            }

            const auto k = juce::String (u.slot) + ":" + juce::String (ledIndex);
            auto prev = lastLedSent.find (k);
            const bool hadOverride = prev != lastLedSent.end() && prev->second.isNotEmpty();
            if (force || (prev == lastLedSent.end() ? want.isNotEmpty() : prev->second != want))
            {
                if (want.isNotEmpty() || hadOverride || force)
                    changes.add (juce::Array<juce::var> { u.slot, ledIndex, want });
                lastLedSent[k] = want;
            }

            auto& ui = ledShown[key (u.uid, c)];
            if (ui != shown) { ui = shown; uiChanged = true; }
        }
    }

    if (! changes.isEmpty())
    {
        auto m = msg ("leds");
        set (m, "l", changes);
        link.send (m);
    }
    if (uiChanged) sendChangeMessage();
}

void ControllerManager::updateScreen (bool force)
{
    const double now = juce::Time::getMillisecondCounterHiRes();
    auto m = msg ("screen");

    const bool tuning = control.isTunerMode() || control.isMuted();
    if (tuning && tuner != nullptr)
    {
        auto& r = tuner->getReading();
        set (m, "mode", "tuner");
        set (m, "note", r.valid ? r.noteName : juce::String());
        set (m, "cents", r.valid ? std::round (r.cents * 2.0f) / 2.0f : 0.0f);
        set (m, "inTune", r.valid && std::abs (r.cents) < 3.0f);
    }
    else
    {
        set (m, "mode", "preset");
        set (m, "top", asciiFold (control.getBankName ? control.getBankName().toUpperCase() : juce::String()));
        set (m, "main", asciiFold (control.getPresetName ? control.getPresetName() : juce::String()));
        set (m, "sub", getInfo().layers > 1 ? "LAYER " + juce::String (getInfo().layer + 1) : juce::String());
    }

    const auto body = juce::JSON::toString (m, true);
    const bool flashing = flashText.isNotEmpty() && now < flashUntil;
    if (! force && body == lastScreen) return;
    if (! force && tuning && now - lastScreenSent < 125.0) return;

    if (flashing && force) set (m, "flash", asciiFold (flashText));
    if (! flashing) flashText.clear();
    lastScreen = body;
    lastScreenSent = now;
    link.send (m);
}

//== Backups =====================================================================
juce::File ControllerManager::getBackupFolder() const
{
    auto name = juce::File::createLegalFileName (getInfo().name + " " + getInfo().id);
    return juce::File::getSpecialLocation (juce::File::userDocumentsDirectory)
               .getChildFile ("PEQUOD Host").getChildFile ("Controller Backups").getChildFile (name);
}

bool ControllerManager::saveBackup (juce::File target)
{
    if (! isConnected()) return false;
    auto root = obj();
    set (root, "pequodControllerBackup", 1);
    set (root, "id", getInfo().id);
    set (root, "name", getInfo().name);
    set (root, "fw", getInfo().fw);
    set (root, "saved", juce::Time::getCurrentTime().toISO8601 (true));
    set (root, "settings", settingsCache);
    juce::Array<juce::var> units;
    for (auto& u : getInfo().units)
    {
        auto e = obj();
        set (e, "uid", u.uid);
        set (e, "slot", u.slot);
        set (e, "kind", u.kind);
        set (e, "cfg", getUnitConfig (u.slot));
        units.add (e);
    }
    set (root, "units", units);

    if (target == juce::File())
    {
        auto folder = getBackupFolder();
        folder.createDirectory();
        target = folder.getChildFile (juce::Time::getCurrentTime().formatted ("%Y-%m-%d %H-%M-%S") + ".json");

        // Keep the 20 newest automatic backups.
        auto files = folder.findChildFiles (juce::File::findFiles, false, "*.json");
        files.sort();
        for (int i = 0; i < files.size() - 19; ++i) files[i].deleteFile();
    }
    return target.replaceWithText (juce::JSON::toString (root));
}

bool ControllerManager::restoreBackup (const juce::File& f, juce::String& report)
{
    auto root = juce::JSON::parse (f);
    if (! root.isObject() || ! root.hasProperty ("pequodControllerBackup")) { report = "That isn't a controller backup."; return false; }
    if (! isConnected()) { report = "Connect the controller first."; return false; }

    int restored = 0, missing = 0;
    if (auto* arr = root["units"].getArray())
        for (auto& e : *arr)
        {
            if (auto* u = getInfo().findUnit (e["uid"].toString()))
            {
                setUnitConfig (u->slot, e["cfg"]);
                ++restored;
            }
            else ++missing;
        }

    auto s = root["settings"];
    if (s.isObject())
    {
        auto patch = obj();
        if (s.hasProperty ("layers")) set (patch, "layers", s["layers"]);
        if (s.hasProperty ("brightness")) set (patch, "brightness", s["brightness"]);
        setSettings (patch);
    }
    report = "Restored " + juce::String (restored) + " unit(s)" + (missing > 0 ? ", " + juce::String (missing) + " not connected" : juce::String()) + ".";
    return true;
}

//== Firmware ====================================================================
bool ControllerManager::hasBundledFirmware()
{
   #if PEQUOD_HAS_FIRMWARE
    return FirmwareData::controller_binSize > 0;
   #else
    return false;
   #endif
}

juce::String ControllerManager::bundledFirmwareVersion()
{
   #if PEQUOD_HAS_FIRMWARE
    return juce::String::fromUTF8 (FirmwareData::version_txt, FirmwareData::version_txtSize).trim();
   #else
    return {};
   #endif
}

int ControllerManager::compareVersions (const juce::String& a, const juce::String& b)
{
    auto pa = juce::StringArray::fromTokens (a.upToFirstOccurrenceOf ("-", false, false), ".", "");
    auto pb = juce::StringArray::fromTokens (b.upToFirstOccurrenceOf ("-", false, false), ".", "");
    for (int i = 0; i < juce::jmax (pa.size(), pb.size()); ++i)
    {
        const int x = pa[i].getIntValue(), y = pb[i].getIntValue();
        if (x != y) return x < y ? -1 : 1;
    }
    return 0;
}

bool ControllerManager::updateAvailable() const
{
    return isConnected() && hasBundledFirmware() && compareVersions (getInfo().fw, bundledFirmwareVersion()) < 0;
}

bool ControllerManager::isUpdating() const   { return updater != nullptr && ! updater->finished; }
float ControllerManager::updateProgress() const { return updater ? updater->progress.load() : 0.0f; }
juce::String ControllerManager::updateStatus() const { return updater ? updater->getStatus() : juce::String(); }

bool ControllerManager::startFirmwareUpdate (juce::String& whyNot)
{
   #if PEQUOD_HAS_FIRMWARE
    if (! isConnected()) { whyNot = "Connect the controller first."; return false; }
    if (isUpdating()) { whyNot = "An update is already running."; return false; }
    if (! getInfo().otaOk)
    {
        whyNot = "This controller was flashed with a partition scheme that has no room for updates. "
                 "Flash it once from the Arduino IDE with Tools > Partition Scheme > \"Minimal SPIFFS (1.9MB APP with OTA)\". "
                 "PEQUOD keeps a backup of its settings so you can restore them afterwards.";
        return false;
    }
    if ((int64_t) FirmwareData::controller_binSize > getInfo().otaMax)
    {
        whyNot = "The new firmware is larger than this controller's update space.";
        return false;
    }
    saveBackup();
    updater = std::make_unique<FirmwareUpdater> (*this, juce::MemoryBlock (FirmwareData::controller_bin, (size_t) FirmwareData::controller_binSize));
    updater->startThread();
    return true;
   #else
    whyNot = "This build of PEQUOD Host doesn't include controller firmware.";
    return false;
   #endif
}

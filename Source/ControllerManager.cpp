#include "ControllerManager.h"
#include "PequodLookAndFeel.h"
#include "PluginColours.h"

namespace
{
    juce::var makeObj() { return juce::var (new juce::DynamicObject()); }

    juce::String key (const juce::String& uid, int code) { return uid + ":" + juce::String (code); }
    juce::String pendingKey (const juce::String& uid, int code, int lay) { return uid + ":" + juce::String (code) + ":" + juce::String (lay); }

    juce::AudioProcessorParameter* findParam (HostEngine& engine, int slotUid, int index)
    {
        if (auto s = engine.findSlot (slotUid))
            if (auto* inst = s->instance())
            {
                auto& params = inst->getParameters();
                if (juce::isPositiveAndBelow (index, params.size()))
                    return params[index];
            }
        return nullptr;
    }

    juce::String upper (const juce::String& s, int maxLen)
    {
        auto t = s.toUpperCase().trim();
        return t.length() > maxLen ? t.substring (0, maxLen) : t;
    }

    juce::String shortName (const juce::String& s, int maxLen)
    {
        auto t = s.trim();
        if (t.length() <= maxLen) return t;
        auto firstWord = t.upToFirstOccurrenceOf (" ", false, false);
        return firstWord.length() <= maxLen ? firstWord : t.substring (0, maxLen);
    }
}

//==============================================================================
ControllerManager::ControllerManager (HostEngine& e, ControlSurface& c, Tuner& t, juce::PropertiesFile& p)
    : engine (e), control (c), tuner (t), props (p), link (e.deviceManager)
{
    link.setManualAddress (props.getValue ("controllerIp"));
    link.addListener (this);
    control.addChangeListener (this);

    // A plugin control learned for a switch: apply the colour / latching chosen before it had a function.
    control.onControllerMappingAdded = [this] (const Mapping& m)
    {
        const auto key = pendingKey (m.ctrlUnit, m.ctrlControl, m.ctrlLayer);
        LearnFor want { m.perPreset, m.toggle, m.ledColour };
        if (auto it = pendingLearn.find (key); it != pendingLearn.end())          // joins the control's other functions
            want = it->second;
        else if (pendingFx.count (key) > 0 || pendingLatch.count (key) > 0)       // chosen before it had a function
            want = { m.perPreset, pendingLatch.count (key) == 0 || pendingLatch[key],
                     (pendingFx.count (key) == 0 || pendingFx[key]) ? Mapping::ledEffectColour : 0u };
        else
            return;
        pendingLearn.erase (key);
        pendingFx.erase (key);
        pendingLatch.erase (key);
        if (want.perPreset == m.perPreset && want.toggle == m.toggle && want.led == m.ledColour) return;

        juce::MessageManager::callAsync ([this, m, want]
        {
            control.removeControllerMapping (m);
            Mapping copy = m;
            copy.perPreset = want.perPreset;
            copy.toggle = want.toggle;
            copy.ledColour = want.led;
            control.setControllerMapping (copy);
        });
    };

    control.onPresetMappingsLoaded = [this] { pushPresetActions(); };

    control.controllerSourceName = [this] (const Mapping& m)
    {
        // short, so it fits the MAPPINGS list: "Switches 1 S2 L1"
        const int c = m.ctrlControl;
        juce::String ctrl = c < 4 ? "S" + juce::String (c + 1)
                          : c == Mapping::ctrlPedal ? juce::String ("Pedal")
                          : c == Mapping::ctrlEncoderTurn ? juce::String ("Turn") : juce::String ("Push");
        if (auto* u = getInfo().findUnit (m.ctrlUnit)) if (u->kind == "pedal_switch" && c == 0) ctrl = "Toe";
        return unitName (m.ctrlUnit) + " " + ctrl + (getNumLayers() > 1 ? " L" + juce::String (m.ctrlLayer + 1) : juce::String());
    };

    startTimerHz (30);
}

ControllerManager::~ControllerManager()
{
    stopTimer();
    if (configDirty) sendConfigNow();
    control.controllerSourceName = nullptr;
    control.onControllerMappingAdded = nullptr;
    control.onPresetMappingsLoaded = nullptr;
    control.removeChangeListener (this);
    link.removeListener (this);
}

void ControllerManager::setManualAddress (const juce::String& ip)
{
    link.setManualAddress (ip);
    props.setValue ("controllerIp", ip.trim());
}

void ControllerManager::status (const juce::String& s)
{
    if (onStatus) onStatus (s);
}

void ControllerManager::sendCmd (const juce::String& type, std::initializer_list<std::pair<const char*, juce::var>> fields)
{
    auto m = makeObj();
    m.getDynamicObject()->setProperty ("t", type);
    for (auto& f : fields) m.getDynamicObject()->setProperty (f.first, f.second);
    link.send (m);
}

//==============================================================================
// Settings
//==============================================================================
juce::var ControllerManager::get (const juce::String& path) const
{
    juce::var v = config;
    for (auto& part : juce::StringArray::fromTokens (path, ".", {}))
    {
        if (auto* arr = v.getArray())
        {
            const int i = part.getIntValue();
            if (! juce::isPositiveAndBelow (i, arr->size())) return {};
            v = arr->getReference (i);
        }
        else if (auto* o = v.getDynamicObject())
            v = o->getProperty (part);
        else
            return {};
    }
    return v;
}

void ControllerManager::set (const juce::String& path, const juce::var& value, bool keep)
{
    if (! config.isObject()) return;
    auto parts = juce::StringArray::fromTokens (path, ".", {});
    if (parts.isEmpty()) return;

    juce::var v = config;
    for (int i = 0; i < parts.size() - 1; ++i)
    {
        juce::var next;
        if (auto* arr = v.getArray())
        {
            const int idx = parts[i].getIntValue();
            while (arr->size() <= idx) arr->add (makeObj());
            next = arr->getReference (idx);
        }
        else if (auto* o = v.getDynamicObject())
        {
            next = o->getProperty (parts[i]);
            if (! next.isObject() && ! next.isArray())
            {
                // "layers" and "layerColors" are lists; everything else is an object
                next = (parts[i] == "layers" && i > 0) || parts[i] == "layerColors" ? juce::var (juce::Array<juce::var>()) : makeObj();
                o->setProperty (parts[i], next);
                next = o->getProperty (parts[i]);
            }
        }
        else return;
        v = next;
    }

    if (auto* arr = v.getArray())
    {
        const int idx = parts[parts.size() - 1].getIntValue();
        while (arr->size() <= idx) arr->add ({});
        arr->set (idx, value);
    }
    else if (auto* o = v.getDynamicObject())
        o->setProperty (parts[parts.size() - 1], value);

    if (keep)
    {
        configDirty = true;
        configDirtySince = juce::Time::getMillisecondCounterHiRes();
    }
    sendChangeMessage();
}

void ControllerManager::sendConfigNow()
{
    configDirty = false;
    if (! config.isObject()) return;
    sendCmd ("set_cfg", { { "cfg", config } });

    // Passwords travel once; after that the controller just says it has one.
    for (auto* k : { "pass", "pass2" })
        if (get (juce::String ("wifi.") + k).isString())
        {
            const bool has = get (juce::String ("wifi.") + k).toString().isNotEmpty();
            if (auto* w = get ("wifi").getDynamicObject())
            {
                w->setProperty (k, juce::var());
                w->setProperty (juce::String (k) == "pass" ? "hasPass" : "hasPass2", has);
            }
        }
}

void ControllerManager::setName (const juce::String& newName)
{
    auto n = newName.trim().substring (0, 24);
    if (n.isEmpty() || n == get ("name").toString()) return;
    set ("name", n);
    sendConfigNow();
    status ("Renaming the controller to \"" + n + "\" - it restarts so the computer sees the new name");
    rebootAt = juce::Time::getMillisecondCounterHiRes() + 700;
}

int ControllerManager::getNumLayers() const
{
    return juce::jlimit (1, 4, hasConfig() ? (int) get ("layers") : getInfo().layers);
}

void ControllerManager::selectLayer (int l)
{
    layer = juce::jlimit (0, getNumLayers() - 1, l);
    sendCmd ("layer", { { "l", layer } });
    updateLeds (true);
    sendChangeMessage();
}

juce::Colour ControllerManager::layerColour (int l) const
{
    static const juce::uint32 defaults[4] = { 0xF44336, 0x9C27B0, 0x2196F3, 0x4CAF50 };
    auto s = get ("layerColors." + juce::String (l)).toString().trimCharactersAtStart ("#");
    return juce::Colour (0xff000000 | (s.isNotEmpty() ? (juce::uint32) s.getHexValue32() : defaults[juce::jlimit (0, 3, l)]));
}

//==============================================================================
// Controls
//==============================================================================
juce::Array<int> ControllerManager::controlCodes (const ControllerUnit& u)
{
    if (u.kind == "footswitch" || u.kind == "button") return { 0, 1, 2, 3 };
    if (u.kind == "pedal")        return { Mapping::ctrlPedal };
    if (u.kind == "pedal_switch") return { Mapping::ctrlPedal, 0 };
    if (u.kind == "encoder")      return { Mapping::ctrlEncoderTurn, Mapping::ctrlEncoderPress };
    return {};
}

juce::String ControllerManager::controlName (const ControllerUnit& u, int code)
{
    if (code == Mapping::ctrlPedal)        return "Pedal";
    if (code == Mapping::ctrlEncoderTurn)  return "Turn";
    if (code == Mapping::ctrlEncoderPress) return "Push";
    if (u.kind == "pedal_switch")          return "Toe switch";
    return "Switch " + juce::String (code + 1);
}

bool ControllerManager::hasLed (const ControllerUnit& u, int code)
{
    return (u.hasButtons() && code >= 0 && code < 4) || (u.kind == "pedal_switch" && code == 0);
}

juce::String ControllerManager::unitName (const juce::String& uid) const
{
    auto n = get ("units." + uid + ".name").toString();
    if (n.isNotEmpty()) return n;
    if (auto* u = getInfo().findUnit (uid)) return u->name;
    return uid;
}

juce::String ControllerManager::describeControl (const juce::String& uid, int code, int lay) const
{
    juce::String ctrl = "Switch " + juce::String (code + 1);
    if (auto* u = getInfo().findUnit (uid)) ctrl = controlName (*u, code);
    else if (code == Mapping::ctrlPedal) ctrl = "Pedal";
    else if (code == Mapping::ctrlEncoderTurn) ctrl = "Turn";
    else if (code == Mapping::ctrlEncoderPress) ctrl = "Push";

    auto s = unitName (uid) + " " + ctrl;
    if (getNumLayers() > 1) s << " L" << (lay + 1);
    return s;
}

juce::String ControllerManager::modeOf (const juce::String& uid, int code, int lay) const
{
    auto m = get (actionPath (uid, code, lay) + ".mode").toString();
    return m.isEmpty() ? juce::String ("pequod") : m;
}

static juce::var makeMidi (const juce::String& type, int num, int ch)
{
    auto v = juce::var (new juce::DynamicObject());
    v.getDynamicObject()->setProperty ("type", type);
    v.getDynamicObject()->setProperty ("num", num);
    v.getDynamicObject()->setProperty ("ch", ch);
    return v;
}

juce::Array<juce::var> ControllerManager::midiOf (const juce::String& uid, int code, int lay) const
{
    juce::Array<juce::var> out;
    auto base = actionPath (uid, code, lay);
    auto list = get (base + ".midi");
    if (control.isControllerPerPreset (uid, code, lay))                 // this preset's own
    {
        auto it = control.presetControllerActions.find (ControlSurface::controllerKey (uid, code, lay));
        list = it != control.presetControllerActions.end() ? it->second["midi"] : juce::var (juce::Array<juce::var>());
        if (! list.isArray()) list = juce::var (juce::Array<juce::var>());
    }
    if (auto* arr = list.getArray())
    {
        for (auto& v : *arr) out.add (makeMidi (v["type"].toString(), (int) v["num"], juce::jlimit (1, 16, (int) v["ch"])));
        return out;
    }
    auto mode = modeOf (uid, code, lay);           // settings from before 2.1: one MIDI message at most
    if (mode == "cc" || mode == "note" || mode == "pc" || mode == "cc_rel" || mode == "cc_abs")
        out.add (makeMidi (mode, (int) get (base + ".num"), juce::jlimit (1, 16, (int) get (base + ".ch"))));
    return out;
}

juce::var ControllerManager::layerOf (const juce::String& uid, int code, int lay) const
{
    auto base = actionPath (uid, code, lay);
    auto l = get (base + ".layer");
    auto midiVar = get (base + ".midi");
    if (control.isControllerPerPreset (uid, code, lay))
    {
        auto it = control.presetControllerActions.find (ControlSurface::controllerKey (uid, code, lay));
        l = it != control.presetControllerActions.end() ? it->second["layer"] : juce::var();
        midiVar = juce::var (juce::Array<juce::var>());
    }
    auto* list = midiVar.getArray();
    auto make = [] (const juce::String& type, int num)
    {
        auto v = juce::var (new juce::DynamicObject());
        v.getDynamicObject()->setProperty ("type", type);
        v.getDynamicObject()->setProperty ("num", juce::jlimit (1, 4, num));
        return v;
    };
    if (l.isObject() || list != nullptr)
        return l.isObject() && l["type"].toString() != "none" && l["type"].toString().isNotEmpty() ? make (l["type"].toString(), (int) l["num"]) : juce::var();
    auto mode = modeOf (uid, code, lay);
    if (mode == "layer_next") return make ("next", 1);
    if (mode == "layer_prev") return make ("prev", 1);
    if (mode == "layer_set")  return make ("set", (int) get (base + ".num"));
    if (mode == "layer_hold") return make ("hold", (int) get (base + ".num"));
    if (mode == "layer")      return make ("turn", 1);
    return {};
}

bool ControllerManager::hasAnything (const juce::String& uid, int code, int lay) const
{
    return mappingFor (uid, code, lay) != nullptr || ! midiOf (uid, code, lay).isEmpty() || ! layerOf (uid, code, lay).isVoid();
}

bool ControllerManager::supportsCombos() const
{
    auto parts = juce::StringArray::fromTokens (getInfo().fw.upToFirstOccurrenceOf ("-", false, false), ".", {});
    const int major = parts[0].getIntValue(), minor = parts[1].getIntValue();
    return major > 2 || (major == 2 && minor >= 1);
}

static juce::var& presetEntry (ControlSurface& control, const juce::String& uid, int code, int lay)
{
    control.markPresetModified();
    auto& e = control.presetControllerActions[ControlSurface::controllerKey (uid, code, lay)];
    if (! e.isObject()) e = juce::var (new juce::DynamicObject());
    return e;
}

void ControllerManager::setScope (const juce::String& uid, int code, int lay, bool perPreset)
{
    if (control.isControllerPerPreset (uid, code, lay) == perPreset) return;
    auto midi = midiOf (uid, code, lay);
    auto l = layerOf (uid, code, lay);
    const bool latch = isLatching (uid, code, lay);
    control.setControllerScope (uid, code, lay, perPreset);
    if (perPreset)
    {
        // the current MIDI / layer settings become this preset's
        auto& e = presetEntry (control, uid, code, lay);
        e.getDynamicObject()->setProperty ("midi", juce::var (midi));
        e.getDynamicObject()->setProperty ("layer", l);
        e.getDynamicObject()->setProperty ("toggle", latch);
        pushPresetActions();
    }
    else
    {
        control.presetControllerActions.erase (ControlSurface::controllerKey (uid, code, lay));
        setMidi (uid, code, lay, midi);
        setLayerAction (uid, code, lay, l);
        set (actionPath (uid, code, lay) + ".toggle", latch);
    }
    sendChangeMessage();
}

void ControllerManager::pushPresetActions()
{
    if (! hasConfig()) return;
    bool any = false;
    for (auto& key : control.perPresetControllerKeys())
    {
        auto parts = juce::StringArray::fromTokens (key, ":", {});
        if (parts.size() != 3) continue;
        const auto uid = parts[0];
        const int code = parts[1].getIntValue(), lay = parts[2].getIntValue();
        if (get ("units." + uid).isVoid()) continue;
        auto it = control.presetControllerActions.find (key);
        auto e = it != control.presetControllerActions.end() ? it->second : juce::var();
        auto base = actionPath (uid, code, lay);
        set (base + ".midi", e["midi"].isArray() ? e["midi"] : juce::var (juce::Array<juce::var>()), false);
        set (base + ".layer", e["layer"].isObject() ? e["layer"] : juce::var (new juce::DynamicObject()), false);
        set (base + ".toggle", e.hasProperty ("toggle") ? (bool) e["toggle"] : true, false);
        set (base + ".mode", mappingFor (uid, code, lay) != nullptr ? juce::String ("pequod") : juce::String ("none"), false);
        any = true;
    }
    if (any && isConnected())
        sendCmd ("set_cfg", { { "cfg", config }, { "temp", true } });
}

void ControllerManager::setMidi (const juce::String& uid, int code, int lay, const juce::Array<juce::var>& list)
{
    if (control.isControllerPerPreset (uid, code, lay))
    {
        presetEntry (control, uid, code, lay).getDynamicObject()->setProperty ("midi", juce::var (list));
        pushPresetActions();
        sendChangeMessage();
        return;
    }
    auto base = actionPath (uid, code, lay);
    auto layerNow = layerOf (uid, code, lay);
    set (base + ".midi", juce::var (list));
    if (get (base + ".layer").isVoid())
        set (base + ".layer", layerNow.isVoid() ? juce::var (new juce::DynamicObject()) : layerNow);
    syncLegacy (uid, code, lay);
}

void ControllerManager::setLayerAction (const juce::String& uid, int code, int lay, const juce::var& l)
{
    if (control.isControllerPerPreset (uid, code, lay))
    {
        presetEntry (control, uid, code, lay).getDynamicObject()->setProperty ("layer", l.isVoid() ? juce::var (new juce::DynamicObject()) : l);
        pushPresetActions();
        sendChangeMessage();
        return;
    }
    auto base = actionPath (uid, code, lay);
    auto midiNow = midiOf (uid, code, lay);
    set (base + ".midi", juce::var (midiNow));
    set (base + ".layer", l.isVoid() ? juce::var (new juce::DynamicObject()) : l);
    syncLegacy (uid, code, lay);
}

/** Older firmware (before 2.1) only reads "mode": give it the closest single thing. */
void ControllerManager::syncLegacy (const juce::String& uid, int code, int lay)
{
    if (! hasConfig()) return;
    if (control.isControllerPerPreset (uid, code, lay)) { pushPresetActions(); return; }
    auto base = actionPath (uid, code, lay);
    auto midi = midiOf (uid, code, lay);
    auto l = layerOf (uid, code, lay);
    juce::String mode = "none";
    if (mappingFor (uid, code, lay) != nullptr) mode = "pequod";
    else if (! midi.isEmpty())
    {
        mode = midi[0]["type"].toString();
        set (base + ".num", midi[0]["num"]);
        set (base + ".ch", midi[0]["ch"]);
    }
    else if (! l.isVoid())
    {
        auto t = l["type"].toString();
        mode = t == "turn" ? juce::String ("layer") : "layer_" + t;
        set (base + ".num", l["num"]);
    }
    else if (get (base + ".midi").isVoid())
        mode = "pequod";                           // nothing at all yet: PEQUOD can still use it
    if (modeOf (uid, code, lay) != mode) set (base + ".mode", mode);
}

const Mapping* ControllerManager::mappingFor (const juce::String& uid, int code, int lay) const
{
    return control.findControllerMapping (uid, code, lay);
}

std::vector<const Mapping*> ControllerManager::mappingsFor (const juce::String& uid, int code, int lay) const
{
    return control.findControllerMappings (uid, code, lay);
}

bool ControllerManager::controlIsOn (const juce::String& uid, int code, int lay) const
{
    for (auto* m : mappingsFor (uid, code, lay))
        if (mappingIsOn (*m)) return true;
    return false;
}

juce::Colour ControllerManager::controlEffectColour (const juce::String& uid, int code, int lay) const
{
    for (auto* m : mappingsFor (uid, code, lay))
        if (auto c = effectColour (*m); ! c.isTransparent()) return c;
    return {};
}

juce::String ControllerManager::shortLabel (const juce::String& uid, int code, int lay) const
{
    juce::StringArray parts;
    for (auto* m : mappingsFor (uid, code, lay)) parts.add (shortLabelOf (*m, code));
    for (auto& mv : midiOf (uid, code, lay))
    {
        auto t = mv["type"].toString();
        const int num = (int) mv["num"];
        parts.add (t == "note" ? juce::MidiMessage::getMidiNoteName (num, true, true, 3)
                 : t == "pc" ? (code == Mapping::ctrlEncoderTurn ? juce::String ("PROGRAM") : "PC " + juce::String (num))
                 : "CC " + juce::String (num));
    }
    auto l = layerOf (uid, code, lay);
    if (! l.isVoid())
    {
        auto t = l["type"].toString();
        parts.add (t == "next" ? juce::String ("LAYER +") : t == "prev" ? juce::String ("LAYER -")
                 : t == "set" ? "LAYER " + l["num"].toString() : t == "hold" ? "HOLD L" + l["num"].toString() : juce::String ("LAYERS"));
    }
    if (parts.isEmpty()) return "-";
    if (parts.size() == 1) return parts[0];
    return parts[0].substring (0, 8) + " +" + juce::String (parts.size() - 1);
}

juce::String ControllerManager::shortLabelOf (const Mapping& mapping, int code) const
{
    {
        auto* m = &mapping;
        switch (m->target)
        {
            case Mapping::Target::bypass:
                if (auto s = engine.findSlot (m->slotUid)) return shortName (s->description.name, 11);
                return "?";
            case Mapping::Target::stompSlot:
            {
                auto slots = engine.getFlatSlots();
                if (m->paramIndex > 0 && m->paramIndex <= (int) slots.size())
                    return shortName (slots[(size_t) m->paramIndex - 1]->description.name, 11);
                return "FX " + juce::String (m->paramIndex);
            }
            case Mapping::Target::param:
                if (auto* p = findParam (engine, m->slotUid, m->paramIndex)) return shortName (p->getName (24), 9);
                return "?";
            case Mapping::Target::presetNext:   return code == Mapping::ctrlEncoderTurn ? "PRESETS" : "NEXT";
            case Mapping::Target::presetPrev:   return "PREV";
            case Mapping::Target::presetSelect: return "P" + juce::String (m->paramIndex);
            case Mapping::Target::bankNext:     return code == Mapping::ctrlEncoderTurn ? "BANKS" : "BANK +";
            case Mapping::Target::bankPrev:     return "BANK -";
            case Mapping::Target::mute:         return "MUTE";
            case Mapping::Target::tuner:        return "TUNER";
            case Mapping::Target::splitMix:     return "SPLIT " + juce::String (engine.getSplitNumber (m->slotUid));
            case Mapping::Target::inputGain:    return "IN";
            case Mapping::Target::outputGain:   return "VOLUME";
        }
        return "?";
    }
}

juce::Colour ControllerManager::effectColour (const Mapping& m) const
{
    switch (m.target)
    {
        case Mapping::Target::bypass:
        case Mapping::Target::param:
            if (auto s = engine.findSlot (m.slotUid)) return PluginColours::forPlugin (s->description);
            return {};
        case Mapping::Target::stompSlot:
        {
            auto slots = engine.getFlatSlots();
            if (m.paramIndex > 0 && m.paramIndex <= (int) slots.size())
                return PluginColours::forPlugin (slots[(size_t) m.paramIndex - 1]->description);
            return {};
        }
        default:
            return {};
    }
}

bool ControllerManager::usesEffectColour (const juce::String& uid, int code, int lay) const
{
    if (auto* m = mappingFor (uid, code, lay)) return m->ledColour == Mapping::ledEffectColour;
    auto it = pendingFx.find (pendingKey (uid, code, lay));
    return it == pendingFx.end() || it->second;
}

void ControllerManager::setUseEffectColour (const juce::String& uid, int code, int lay, bool on)
{
    if (mappingFor (uid, code, lay) != nullptr) control.setControllerLedColour (uid, code, lay, on ? Mapping::ledEffectColour : 0);
    else pendingFx[pendingKey (uid, code, lay)] = on;
    updateLeds (true);
    sendChangeMessage();
}

bool ControllerManager::isLatching (const juce::String& uid, int code, int lay) const
{
    if (auto* m = mappingFor (uid, code, lay)) return m->toggle;
    if (! midiOf (uid, code, lay).isEmpty())
    {
        if (control.isControllerPerPreset (uid, code, lay))
        {
            auto it = control.presetControllerActions.find (ControlSurface::controllerKey (uid, code, lay));
            return it == control.presetControllerActions.end() || ! it->second.hasProperty ("toggle") || (bool) it->second["toggle"];
        }
        return get (actionPath (uid, code, lay) + ".toggle").isVoid() || (bool) get (actionPath (uid, code, lay) + ".toggle");
    }
    auto it = pendingLatch.find (pendingKey (uid, code, lay));
    return it == pendingLatch.end() || it->second;
}

void ControllerManager::setLatching (const juce::String& uid, int code, int lay, bool latching)
{
    if (control.isControllerPerPreset (uid, code, lay))
    {
        presetEntry (control, uid, code, lay).getDynamicObject()->setProperty ("toggle", latching);
        pushPresetActions();
    }
    else
        set (actionPath (uid, code, lay) + ".toggle", latching);      // the MIDI it sends
    auto all = mappingsFor (uid, code, lay);
    if (! all.empty())
    {
        std::vector<Mapping> copies;
        for (auto* m : all) copies.push_back (*m);
        for (auto& c : copies) { c.toggle = latching; control.setControllerMapping (c); }
    }
    else
    {
        pendingLatch[pendingKey (uid, code, lay)] = latching;
        sendChangeMessage();
    }
}

juce::Colour ControllerManager::colourOf (const juce::String& uid, int code) const
{
    if (usesEffectColour (uid, code, layer))
        if (auto c = controlEffectColour (uid, code, layer); ! c.isTransparent())
            return PluginColours::forLed (c);

    auto s = get (controlPath (uid, code) + ".color").toString();
    if (s == "layer") return layerColour (layer);
    s = s.trimCharactersAtStart ("#");
    return s.isEmpty() ? Pequod::Colours::mustard : juce::Colour (0xff000000 | (juce::uint32) s.getHexValue32());
}

bool ControllerManager::isPressed (const juce::String& uid, int code) const
{
    auto it = pressed.find (key (uid, code));
    return it != pressed.end() && it->second;
}

float ControllerManager::pedalValue (const juce::String& uid) const
{
    auto it = pedals.find (uid);
    return it != pedals.end() ? it->second : 0.0f;
}

int ControllerManager::pedalRaw (const juce::String& uid) const
{
    auto it = raws.find (uid);
    return it != raws.end() ? it->second : -1;
}

void ControllerManager::watchPedal (const juce::String& uid, bool on)
{
    if (on) watched.insert (uid); else watched.erase (uid);
    sendCmd ("cal", { { "u", uid }, { "on", on } });
}

bool ControllerManager::mappingIsOn (const Mapping& m) const
{
    switch (m.target)
    {
        case Mapping::Target::bypass:
            if (auto s = engine.findSlot (m.slotUid)) return ! s->bypassed;
            return false;
        case Mapping::Target::stompSlot:
        {
            auto slots = engine.getFlatSlots();
            return m.paramIndex > 0 && m.paramIndex <= (int) slots.size() && ! slots[(size_t) m.paramIndex - 1]->bypassed;
        }
        case Mapping::Target::param:
            if (auto* p = findParam (engine, m.slotUid, m.paramIndex)) return p->getValue() >= 0.5f;
            return false;
        case Mapping::Target::splitMix:
            if (auto* item = engine.findItem (m.slotUid)) return item->mix >= 0.5f;
            return false;
        case Mapping::Target::mute:         return control.isMuted() && ! control.isTunerMode();
        case Mapping::Target::tuner:        return control.isTunerMode();
        case Mapping::Target::presetSelect: return control.getPresetIndex && control.getPresetIndex() == m.paramIndex - 1;
        case Mapping::Target::presetNext: case Mapping::Target::presetPrev:
        case Mapping::Target::bankNext:   case Mapping::Target::bankPrev:
        case Mapping::Target::inputGain:  case Mapping::Target::outputGain:
            return false;
    }
    return false;
}

bool ControllerManager::isLit (const juce::String& uid, int code) const
{
    if (isPressed (uid, code)) return true;
    if (mappingFor (uid, code, layer) != nullptr && controlIsOn (uid, code, layer)) return true;
    auto led = get (controlPath (uid, code) + ".led").toString();
    return led == "on";
}

//==============================================================================
// PEQUOD functions
//==============================================================================
void ControllerManager::assign (const juce::String& uid, int code, int lay, Mapping::Target target, int uidOrIndex, const Mapping* replace)
{
    std::optional<Mapping> old;
    if (replace != nullptr) old = *replace;
    auto others = mappingsFor (uid, code, lay);
    const Mapping* like = nullptr;                 // the control's settings come from its other functions
    for (auto* o : others)
        if (! old.has_value() || ! o->sameFunction (*old)) { like = o; break; }
    if (like == nullptr && old.has_value()) like = &*old;

    const auto key = pendingKey (uid, code, lay);
    Mapping m;
    m.source = Mapping::Source::controller;
    m.ctrlUnit = uid;
    m.ctrlControl = code;
    m.ctrlLayer = lay;
    m.target = target;
    if (target == Mapping::Target::stompSlot || target == Mapping::Target::presetSelect) m.paramIndex = uidOrIndex;
    else m.slotUid = uidOrIndex;

    if (like != nullptr)
    {
        m.perPreset = like->perPreset;
        m.toggle = like->toggle;
        m.ledColour = like->ledColour;
    }
    else
    {
        m.perPreset = target == Mapping::Target::param || target == Mapping::Target::bypass || target == Mapping::Target::splitMix;
        m.toggle = pendingLatch.count (key) == 0 || pendingLatch[key];
        m.ledColour = (pendingFx.count (key) == 0 || pendingFx[key]) ? Mapping::ledEffectColour : 0;
    }
    pendingFx.erase (key);
    pendingLatch.erase (key);

    if (old.has_value()) control.removeControllerMapping (*old);
    control.setControllerMapping (m);
    syncLegacy (uid, code, lay);
    updateLeds (true);
}

void ControllerManager::removeFunction (const Mapping& m)
{
    const Mapping copy = m;
    control.removeControllerMapping (copy);
    syncLegacy (copy.ctrlUnit, copy.ctrlControl, copy.ctrlLayer);
    updateLeds (true);
}

void ControllerManager::unassign (const juce::String& uid, int code, int lay)
{
    control.clearControllerMapping (uid, code, lay);
    syncLegacy (uid, code, lay);
    updateLeds (true);
}

void ControllerManager::learnPluginControl (const juce::String& uid, int code, int lay, const Mapping* replace)
{
    if (modeOf (uid, code, lay) == "none") set (actionPath (uid, code, lay) + ".mode", "pequod");
    std::optional<Mapping> old;
    if (replace != nullptr) old = *replace;
    if (old.has_value()) control.removeControllerMapping (*old);

    auto others = mappingsFor (uid, code, lay);
    if (! others.empty())
        pendingLearn[pendingKey (uid, code, lay)] = { others.front()->perPreset, others.front()->toggle, others.front()->ledColour };
    else if (old.has_value())
        pendingLearn[pendingKey (uid, code, lay)] = { old->perPreset, old->toggle, old->ledColour };
    control.armLearnForController (uid, code, lay, describeControl (uid, code, lay));
}

//==============================================================================
// Maintenance
//==============================================================================
void ControllerManager::identify (const juce::String& uid, int code) { sendCmd ("identify", { { "u", uid }, { "c", code } }); }
void ControllerManager::rescan()       { sendCmd ("rescan"); status ("Looking for boards on the controller..."); }
void ControllerManager::reboot()       { if (configDirty) sendConfigNow(); sendCmd ("reboot"); }
void ControllerManager::factoryReset() { sendCmd ("factory"); status ("Controller reset to factory settings - it restarts now"); }

void ControllerManager::scanWifi()
{
    wifiScanning = true;
    sendCmd ("wifiscan");
    sendChangeMessage();
}

juce::File ControllerManager::bundledFirmwareFolder()
{
    return juce::File::getSpecialLocation (juce::File::currentExecutableFile).getSiblingFile ("Controller firmware");
}

juce::String ControllerManager::bundledFirmwareVersion()
{
    return bundledFirmwareFolder().getChildFile ("version.txt").loadFileAsString().trim();
}

bool ControllerManager::newerFirmwareAvailable() const
{
    auto bundled = bundledFirmwareVersion();
    if (bundled.isEmpty() || ! isConnected() || getInfo().fw.isEmpty()) return false;
    auto num = [] (const juce::String& v)
    {
        auto p = juce::StringArray::fromTokens (v.upToFirstOccurrenceOf ("-", false, false), ".", {});
        return p[0].getIntValue() * 1000000 + p[1].getIntValue() * 1000 + p[2].getIntValue();
    };
    return bundledFirmwareFolder().getChildFile ("PEQUOD_FC.bin").existsAsFile() && num (bundled) > num (getInfo().fw);
}

bool ControllerManager::startFirmwareUpdate (const juce::File& file, juce::String& whyNot)
{
    if (! isConnected()) { whyNot = "The controller isn't connected."; return false; }
    if (fw.active)       { whyNot = "An update is already running."; return false; }

    juce::MemoryBlock image;
    if (! file.loadFileAsData (image) || image.getSize() < 1024) { whyNot = "That file couldn't be read."; return false; }
    if (((const uint8_t*) image.getData())[0] != 0xE9)
    {
        whyNot = "That isn't an ESP32 firmware file. Use PEQUOD_FC.bin (not the -full.bin).";
        return false;
    }
    if (! getInfo().otaOk)
    {
        whyNot = "This controller was flashed with a partition scheme that has no room for updates. Flash it once with USB from the Arduino IDE.";
        return false;
    }
    if (getInfo().otaMax > 0 && (int64_t) image.getSize() > getInfo().otaMax)
    {
        whyNot = "That firmware is too big for this controller.";
        return false;
    }

    fw = {};
    fw.active = true;
    fw.image = std::move (image);
    fw.total = (int) ((fw.image.getSize() + fwChunk - 1) / fwChunk);
    fw.waitingBegin = true;
    fw.sentAt = juce::Time::getMillisecondCounterHiRes();
    fw.status = "Starting update...";
    sendCmd ("fw_begin", { { "size", (int) fw.image.getSize() } });
    sendChangeMessage();
    return true;
}

void ControllerManager::sendFirmwareChunk()
{
    const size_t off = (size_t) fw.next * fwChunk;
    const size_t n = juce::jmin ((size_t) fwChunk, fw.image.getSize() - off);
    sendCmd ("fw_data", { { "n", fw.next },
                          { "d", juce::Base64::toBase64 ((const char*) fw.image.getData() + off, n) } });
    fw.sentAt = juce::Time::getMillisecondCounterHiRes();
}

void ControllerManager::finishFirmware (const juce::String& s)
{
    fw.active = false;
    fw.image.reset();
    fw.status = s;
    status (s);
    sendChangeMessage();
}

//==============================================================================
// Link events
//==============================================================================
void ControllerManager::controllerConnected()
{
    layer = getInfo().layer;
    config = juce::var();
    pressed.clear();
    networks.clear();
    lastLeds.clear();
    lastScreen.clear();
    sendCmd ("get_cfg");
    for (auto& uid : watched) sendCmd ("cal", { { "u", uid }, { "on", true } });
    status ("Foot controller \"" + getInfo().name + "\" connected (" + describeConnection() + ")"
            + (newerFirmwareAvailable() ? " - firmware " + bundledFirmwareVersion() + " is ready to install under CONTROLLER > SETTINGS" : juce::String()));
    sendChangeMessage();
}

void ControllerManager::controllerDisconnected()
{
    if (fw.active && fw.next < fw.total) finishFirmware ("Firmware update interrupted - the controller kept its old firmware");
    config = juce::var();
    configDirty = false;
    pressed.clear();
    wifiScanning = false;
    status ("Foot controller disconnected");
    sendChangeMessage();
}

void ControllerManager::controllerInfoChanged()
{
    layer = getInfo().layer;
    sendChangeMessage();
}

void ControllerManager::controllerMessage (const juce::String& type, const juce::var& msg)
{
    if (type == "cfg")
    {
        config = msg["cfg"];
        configDirty = false;
        pushPresetActions();          // the loaded preset's own switch settings
        updateLeds (true);
        updateScreen (true);
        sendChangeMessage();
    }
    else if (type == "ev")
    {
        handleEvent (msg);
    }
    else if (type == "press")
    {
        const auto uid = msg["u"].toString();
        const int code = (int) msg["c"];
        const bool down = (int) msg["v"] != 0;
        pressed[key (uid, code)] = down;
        if (down && onControlPressed) onControlPressed (uid, code);
    }
    else if (type == "layer")
    {
        layer = juce::jlimit (0, 3, (int) msg["l"]);
        updateLeds (true);
        updateScreen (true);
        sendChangeMessage();
    }
    else if (type == "raw")
    {
        raws[msg["u"].toString()] = (int) msg["raw"];
    }
    else if (type == "wifilist")
    {
        networks.clear();
        if (auto* arr = msg["nets"].getArray())
            for (auto& n : *arr)
            {
                Network net { n["ssid"].toString(), (int) n["rssi"], (bool) n["lock"] };
                if (net.ssid.isEmpty()) continue;
                auto dup = std::find_if (networks.begin(), networks.end(), [&] (auto& x) { return x.ssid == net.ssid; });
                if (dup == networks.end()) networks.push_back (net);
            }
        std::sort (networks.begin(), networks.end(), [] (auto& a, auto& b) { return a.rssi > b.rssi; });
        wifiScanning = false;
        sendChangeMessage();
    }
    else if (type == "fw_ack" && fw.active)
    {
        const int n = (int) msg["n"];
        if (! (bool) msg["ok"])
        {
            finishFirmware ("Firmware update failed: " + msg["err"].toString());
            sendCmd ("fw_abort");
            return;
        }
        if (fw.waitingBegin && n == -1) { fw.waitingBegin = false; fw.next = 0; }
        else if (n == fw.next)          { fw.next++; fw.retries = 0; }
        else return;

        if (fw.next < fw.total)
        {
            sendFirmwareChunk();
            const int pct = fw.next * 100 / fw.total;
            fw.status = "Updating firmware... " + juce::String (pct) + "%";
        }
        else
        {
            fw.status = "Checking the new firmware...";
            sendCmd ("fw_end");
            fw.sentAt = juce::Time::getMillisecondCounterHiRes();
        }
        sendChangeMessage();
    }
    else if (type == "fw_done")
    {
        if ((bool) msg["ok"]) finishFirmware ("Firmware updated - the controller is restarting");
        else                  finishFirmware ("Firmware update failed: " + msg["err"].toString());
    }
    else if (type == "err")
    {
        status ("Controller: " + msg["msg"].toString());
    }
}

void ControllerManager::handleEvent (const juce::var& msg)
{
    const auto uid = msg["u"].toString();
    const int code = (int) msg["c"];
    const int lay = (int) msg["l"];
    float value = (float) (double) msg["v"];
    int delta = 0;

    if (code == Mapping::ctrlPedal)
    {
        value = msg.hasProperty ("f") ? (float) (double) msg["f"] : value / 127.0f;
        pedals[uid] = value;
    }
    else if (code == Mapping::ctrlEncoderTurn)
    {
        delta = (int) value;
        value = 0.0f;
        if (delta == 0) return;
    }

    const bool hit = control.handleControllerEvent (uid, code, lay, value, delta);
    if (! hit) return;

    {
        const bool isPressEvent = isSwitch (code) && value >= 0.5f;
        const double now = juce::Time::getMillisecondCounterHiRes();
        juce::StringArray parts;
        for (auto* m : mappingsFor (uid, code, lay))
        {
            if (isPressEvent || (! isSwitch (code) && m->target == Mapping::Target::param))
            {
                auto f = flashFor (*m, isPressEvent);
                if (f.isNotEmpty()) parts.add (f);
            }
        }
        if (! parts.isEmpty() && (isPressEvent || now - lastParamFlash > 60))
        {
            if (! isPressEvent) lastParamFlash = now;
            pendingFlash = parts.size() == 1 ? parts[0] : parts[0].substring (0, 12) + " +" + juce::String (parts.size() - 1);
        }
    }
    updateLeds (false);
}

juce::String ControllerManager::flashFor (const Mapping& m, bool afterPress) const
{
    juce::ignoreUnused (afterPress);
    switch (m.target)
    {
        case Mapping::Target::bypass:
            if (auto s = engine.findSlot (m.slotUid)) return upper (s->description.name, 14) + (s->bypassed ? " OFF" : " ON");
            return {};
        case Mapping::Target::stompSlot:
        {
            auto slots = engine.getFlatSlots();
            if (m.paramIndex > 0 && m.paramIndex <= (int) slots.size())
            {
                auto& s = slots[(size_t) m.paramIndex - 1];
                return upper (s->description.name, 14) + (s->bypassed ? " OFF" : " ON");
            }
            return "NO EFFECT " + juce::String (m.paramIndex);
        }
        case Mapping::Target::param:
            if (auto* p = findParam (engine, m.slotUid, m.paramIndex))
                return upper (p->getName (12), 12) + " " + p->getCurrentValueAsText().substring (0, 8);
            return {};
        case Mapping::Target::splitMix:
            if (auto* item = engine.findItem (m.slotUid)) return item->mix >= 0.5f ? "SPLIT B" : "SPLIT A";
            return {};
        case Mapping::Target::mute:
            return control.isMuted() ? "MUTED" : "UNMUTED";
        default:
            return {};
    }
}

void ControllerManager::changeListenerCallback (juce::ChangeBroadcaster*)
{
    // mappings changed (learn finished, preset loaded...)
    updateLeds (true);
    sendChangeMessage();
}

//==============================================================================
// Feedback to the controller
//==============================================================================
void ControllerManager::updateLeds (bool force)
{
    if (! isConnected() || ! hasConfig()) return;

    auto list = juce::Array<juce::var>();
    juce::String sig;
    for (auto& u : getInfo().units)
        for (int code : controlCodes (u))
        {
            if (! hasLed (u, code)) continue;
            auto* m = mappingFor (u.uid, code, layer);
            if (m == nullptr) continue;
            const bool on = controlIsOn (u.uid, code, layer);
            juce::Colour fx;
            if (m->ledColour == Mapping::ledEffectColour) fx = controlEffectColour (u.uid, code, layer);
            if (fx.isTransparent())
            {
                if (! on) continue;
                list.add (juce::Array<juce::var> { u.uid, code, 1 });
                sig << u.uid << code << ";";
            }
            else
            {
                // effect colour: sent even when off, so the dim glow is the effect's colour too
                auto hex = "#" + PluginColours::forLed (fx).toDisplayString (false);
                list.add (juce::Array<juce::var> { u.uid, code, on ? 1 : 0, hex });
                sig << u.uid << code << (on ? "+" : "-") << hex << ";";
            }
        }

    const double now = juce::Time::getMillisecondCounterHiRes();
    if (! force && sig == lastLeds && now - lastLedsSent < 2500) return;   // resend now and then (Wi-Fi can drop a packet)
    lastLeds = sig;
    lastLedsSent = now;
    sendCmd ("leds", { { "l", list } });
}

void ControllerManager::updateScreen (bool force)
{
    if (! isConnected() || fw.active) return;

    const bool tunerWhenMuted = get ("display.tunerWhenMuted").isVoid() || (bool) get ("display.tunerWhenMuted");
    const bool showTuner = control.isTunerMode() || (control.isMuted() && tunerWhenMuted);

    auto m = makeObj();
    auto* o = m.getDynamicObject();
    o->setProperty ("t", "screen");
    if (showTuner)
    {
        auto& r = tuner.getReading();
        o->setProperty ("mode", "tuner");
        o->setProperty ("top", control.isTunerMode() ? "TUNER" : "MUTED");
        o->setProperty ("note", r.valid ? r.noteName : juce::String());
        o->setProperty ("cents", r.valid ? juce::roundToInt (r.cents * 10.0f) / 10.0 : 0.0);
        o->setProperty ("inTune", r.valid && std::abs (r.cents) < 3.0f);
    }
    else
    {
        auto bank = control.getBankName ? control.getBankName() : juce::String();
        auto preset = control.getPresetName ? control.getPresetName() : juce::String();
        const int idx = control.getPresetIndex ? control.getPresetIndex() : -1;
        juce::String sub;
        if (idx >= 0) sub << "PRESET " << (idx + 1);
        if (getNumLayers() > 1) sub << (sub.isEmpty() ? "" : "   ") << "LAYER " << (layer + 1);
        o->setProperty ("mode", "preset");
        o->setProperty ("top", upper (bank, 21));
        o->setProperty ("main", upper (preset, 20));
        o->setProperty ("sub", sub);
    }

    const auto sig = juce::JSON::toString (m, true);
    const double now = juce::Time::getMillisecondCounterHiRes();
    const bool changed = sig != lastScreen;

    if (pendingFlash.isNotEmpty())
    {
        o->setProperty ("flash", pendingFlash);
        pendingFlash.clear();
    }
    else
    {
        if (! force && ! changed && now - lastScreenSent < 3000) return;
        if (! force && showTuner && now - lastScreenSent < 66) return;     // tuner: ~15 updates a second
    }

    lastScreen = sig;
    lastScreenSent = now;
    link.send (m);
}

void ControllerManager::timerCallback()
{
    const double now = juce::Time::getMillisecondCounterHiRes();

    if (configDirty && now - configDirtySince > 120)
        sendConfigNow();

    if (! isConnected()) return;

    if (rebootAt > 0 && now > rebootAt) { rebootAt = 0; reboot(); }

    if (fw.active)
    {
        if (now - fw.sentAt > 3000)
        {
            if (++fw.retries > 5) { finishFirmware ("Firmware update failed: the controller stopped answering"); return; }
            if (fw.waitingBegin)            sendCmd ("fw_begin", { { "size", (int) fw.image.getSize() } });
            else if (fw.next < fw.total)    sendFirmwareChunk();
            else                            sendCmd ("fw_end");
            fw.sentAt = now;
        }
        return;
    }

    updateLeds (false);
    updateScreen (false);
}

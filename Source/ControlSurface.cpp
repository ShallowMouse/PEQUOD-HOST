#include "ControlSurface.h"
#include <algorithm>
#include <map>

namespace
{
    const char* targetNames[] = { "param", "bypass", "splitMix", "inputGain", "outputGain", "presetNext", "presetPrev", "mute", "bankNext", "bankPrev", "tuner", "stompSlot", "presetSelect", "tapTempo", "tempo", "click", "instrument" };
    const char* sourceNames[] = { "cc", "note", "osc", "ctrl" };

    template <typename Enum>
    Enum enumFromName (const juce::String& s, const char* const* names, int count, Enum fallback)
    {
        for (int i = 0; i < count; ++i)
            if (s == names[i]) return (Enum) i;
        return fallback;
    }

    float argAsFloat (const juce::OSCMessage& m, float fallback)
    {
        if (m.isEmpty()) return fallback;
        auto& a = m[0];
        if (a.isFloat32()) return a.getFloat32();
        if (a.isInt32())   return (float) a.getInt32();
        return fallback;
    }

}

//==============================================================================
ControlSurface::ControlSurface (HostEngine& e) : engine (e)
{
    receiver.addListener (this);
    engine.setParamObserver (this);
    engine.addChangeListener (this);
    engine.deviceManager.addMidiInputDeviceCallback ({}, this);
    startTimerHz (30);
}

ControlSurface::~ControlSurface()
{
    stopTimer();
    engine.deviceManager.removeMidiInputDeviceCallback ({}, this);
    engine.removeChangeListener (this);
    engine.setParamObserver (nullptr);
    receiver.removeListener (this);
    receiver.disconnect();
    sender.disconnect();
}

//==============================================================================
bool ControlSurface::setOscInputPort (int port)
{
    receiver.disconnect();
    oscInPort = port;
    oscInOk = port > 0 && receiver.connect (port);
    sendChangeMessage();
    return oscInOk;
}

bool ControlSurface::setOscOutput (const juce::String& host, int port)
{
    sender.disconnect();
    oscOutHost = host;
    oscOutPort = port;
    oscOutOk = host.isNotEmpty() && port > 0 && sender.connect (host, port);
    sendChangeMessage();
    if (oscOutOk) sendRackSummary();
    return oscOutOk;
}

void ControlSurface::send (const juce::String& address, float v)
{
    if (oscOutOk) sender.send (juce::OSCAddressPattern (address), v);
}

void ControlSurface::send (const juce::String& address, int v)
{
    if (oscOutOk) sender.send (juce::OSCAddressPattern (address), (juce::int32) v);
}

void ControlSurface::send (const juce::String& address, const juce::String& s)
{
    if (oscOutOk) sender.send (juce::OSCAddressPattern (address), s);
}

//==============================================================================
juce::AudioProcessorParameter* ControlSurface::getParam (int slotUid, int index) const
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

void ControlSurface::pluginParameterChanged (int slotUid, int paramIndex, float value)
{
    // May be called on the audio thread: no allocation, no locks.
    const auto scope = fifo.write (1);
    if (scope.blockSize1 > 0)      fifoData[(size_t) scope.startIndex1] = { slotUid, paramIndex, value };
    else if (scope.blockSize2 > 0) fifoData[(size_t) scope.startIndex2] = { slotUid, paramIndex, value };

    if (learnState == LearnState::waitingForTarget
         && juce::Time::getMillisecondCounterHiRes() - learnArmedTime > 300.0)
        learnCandidate = ((juce::int64) slotUid << 32) | (juce::int64) (juce::uint32) paramIndex;
}

void ControlSurface::pluginParameterGesture (int slotUid, int paramIndex)
{
    if (learnState == LearnState::waitingForTarget)
        learnCandidate = ((juce::int64) slotUid << 32) | (juce::int64) (juce::uint32) paramIndex;
}

//==============================================================================
void ControlSurface::timerCallback()
{
    // 1. Learn: a plugin control was touched.
    if (learnState == LearnState::waitingForTarget)
    {
        auto c = learnCandidate.exchange (-1);
        if (c >= 0)
        {
            const bool fromController = pendingHasControllerSource;
            if (! fromController) pending = {};
            pending.target = Mapping::Target::param;
            pending.slotUid = (int) (c >> 32);
            pending.paramIndex = (int) (c & 0xffffffff);
            if (getParam (pending.slotUid, pending.paramIndex) != nullptr)
            {
                if (fromController)
                {
                    pendingHasControllerSource = false;
                    learnState = LearnState::off;
                    Mapping m = pending;
                    m.toggle = true;
                    m.perPreset = true;          // plugin knobs: saved with the preset unless changed
                    m.ledColour = Mapping::ledEffectColour;
                    setControllerMapping (m);
                }
                else
                {
                    learnState = LearnState::waitingForSource;
                    sendChangeMessage();
                }
            }
        }
    }

    // 2. Feedback: drain parameter changes, newest value wins.
    std::map<juce::int64, ParamEvent> latest;
    {
        const auto scope = fifo.read (fifo.getNumReady());
        auto take = [&] (int start, int size)
        {
            for (int i = 0; i < size; ++i)
            {
                auto& ev = fifoData[(size_t) (start + i)];
                latest[((juce::int64) ev.slotUid << 32) | (juce::uint32) ev.index] = ev;
            }
        };
        take (scope.startIndex1, scope.blockSize1);
        take (scope.startIndex2, scope.blockSize2);
    }

    if (oscOutOk && ! latest.empty())
    {
        for (auto& [key, ev] : latest)
        {
            if (auto n = engine.getFlatIndex (ev.slotUid))
                send ("/pequod/slot/" + juce::String (n) + "/param/" + juce::String (ev.index + 1), ev.value);

            for (auto& m : mappings)
                if (m.source == Mapping::Source::osc && m.target == Mapping::Target::param
                     && m.slotUid == ev.slotUid && m.paramIndex == ev.index)
                    send (m.oscAddress, ev.value);
        }
    }

    // 3. Feedback for host-level values (polled so UI, MIDI and OSC changes are all covered).
    static double lastBpm = -1;
    if (std::abs (engine.transport.bpm.load() - lastBpm) > 0.01) { lastBpm = engine.transport.bpm.load(); send ("/pequod/tempo", (float) lastBpm); }
    static float lastIn = -999, lastOut = -999;
    auto inDb = engine.getInputStage()->gainDb.load();
    auto outDb = engine.getOutputStage()->gainDb.load();
    if (std::abs (inDb - lastIn) > 0.001f)   { lastIn = inDb;   send ("/pequod/master/in",  dbToNorm (inDb)); }
    if (std::abs (outDb - lastOut) > 0.001f) { lastOut = outDb; send ("/pequod/master/out", dbToNorm (outDb)); }

    // Mute + tuner feedback
    const int muteNow = isMuted() ? 1 : 0;
    if (muteNow != lastMuteSent)
    {
        lastMuteSent = muteNow;
        send ("/pequod/mute", muteNow);
        for (auto& m : mappings)
            if (m.source == Mapping::Source::osc && m.target == Mapping::Target::mute)
                send (m.oscAddress, (float) muteNow);
    }

    if (tuner != nullptr && ++tunerTick % 3 == 0)
    {
        auto& r = tuner->getReading();
        auto note = r.valid ? r.noteName : juce::String ("-");
        if (r.valid || note != lastTunerNote)
        {
            send ("/pequod/tuner/note", note);
            if (r.valid)
            {
                send ("/pequod/tuner/cents", r.cents);
                send ("/pequod/tuner/hz", r.frequency);
            }
        }
        lastTunerNote = note;
    }

    int k = 0;
    for (auto* split : engine.getSplits())
    {
        if (k >= (int) lastMix.size()) lastMix.push_back (-1.0f);
        if (std::abs (split->mix - lastMix[(size_t) k]) > 0.0005f)
        {
            lastMix[(size_t) k] = split->mix;
            send ("/pequod/split/" + juce::String (k + 1) + "/mix", split->mix);
            for (auto& m : mappings)
                if (m.source == Mapping::Source::osc && m.target == Mapping::Target::splitMix && m.slotUid == split->uid)
                    send (m.oscAddress, split->mix);
        }
        ++k;
    }
}

void ControlSurface::setMuted (bool shouldMute)
{
    engine.getOutputStage()->muted = shouldMute;
    sendChangeMessage();
}

void ControlSurface::changeListenerCallback (juce::ChangeBroadcaster*)
{
    resolveGlobalMappings();
    sendRackSummary();
}

void ControlSurface::sendRackSummary()
{
    if (! oscOutOk) return;

    auto slots = engine.getFlatSlots();
    send ("/pequod/rack/count", (int) slots.size());

    int n = 1;
    for (auto& s : slots)
    {
        auto base = "/pequod/slot/" + juce::String (n++);
        send (base + "/name", s->description.name);
        send (base + "/on", s->bypassed ? 0 : 1);
        if (auto* inst = s->instance())
            send (base + "/params", inst->getParameters().size());
    }

    int k = 1;
    for (auto* split : engine.getSplits())
        send ("/pequod/split/" + juce::String (k++) + "/mix", split->mix);

    if (getBankName) send ("/pequod/bank/name", getBankName());
    if (getBankIndex) send ("/pequod/bank/index", getBankIndex() + 1);
    if (getPresetName) send ("/pequod/preset/name", getPresetName());
    if (getPresetIndex) send ("/pequod/preset/index", getPresetIndex() + 1);
}

void ControlSurface::sendFullState()
{
    if (! oscOutOk) return;
    sendRackSummary();

    int n = 1;
    for (auto& s : engine.getFlatSlots())
    {
        auto base = "/pequod/slot/" + juce::String (n++) + "/param/";
        if (auto* inst = s->instance())
        {
            auto& params = inst->getParameters();
            for (int i = 0; i < juce::jmin (256, params.size()); ++i)
            {
                send (base + juce::String (i + 1), params[i]->getValue());
                send (base + juce::String (i + 1) + "/name", params[i]->getName (64));
            }
        }
    }
    send ("/pequod/master/in",  dbToNorm (engine.getInputStage()->gainDb));
    send ("/pequod/master/out", dbToNorm (engine.getOutputStage()->gainDb));
}

//==============================================================================
void ControlSurface::oscBundleReceived (const juce::OSCBundle& bundle)
{
    for (auto& el : bundle)
    {
        if (el.isMessage())     oscMessageReceived (el.getMessage());
        else if (el.isBundle()) oscBundleReceived (el.getBundle());
    }
}

void ControlSurface::oscMessageReceived (const juce::OSCMessage& msg)
{
    lastOscActivity = juce::Time::getMillisecondCounterHiRes();
    const auto address = msg.getAddressPattern().toString();
    const float value = argAsFloat (msg, 1.0f);

    if (learnState == LearnState::waitingForSource && address != "/pequod/sync")
    {
        finishLearn (Mapping::Source::osc, 0, 0, address);
        return;
    }

    bool handled = false;
    for (auto& m : mappings)
        if (m.source == Mapping::Source::osc && m.oscAddress == address && ! isOverridden (m))
        {
            apply (m, juce::jlimit (0.0f, 1.0f, value), value >= 0.5f);
            handled = true;
        }

    if (! handled)
        handleFixedOsc (msg, value);
}

bool ControlSurface::handleFixedOsc (const juce::OSCMessage& msg, float value)
{
    auto parts = juce::StringArray::fromTokens (msg.getAddressPattern().toString(), "/", "");
    parts.removeEmptyStrings();
    if (parts.size() < 2 || parts[0] != "pequod") return false;

    const bool press = value >= 0.5f;

    if (parts[1] == "sync") { sendFullState(); return true; }
    if (parts[1] == "tap")  { if (msg.isEmpty() || press) { engine.transport.tap(); if (onTempoTapped) onTempoTapped(); } return true; }
    if (parts[1] == "tempo" && ! msg.isEmpty()) { engine.transport.setBpm (argAsFloat (msg, 120.0f)); return true; }
    if (parts[1] == "click") { engine.transport.clickOn = msg.isEmpty() ? ! engine.transport.clickOn.load() : press; return true; }
    if (parts[1] == "instrument")
    {
        auto& in = engine.getRack().instrumentInput;
        in = msg.isEmpty() ? ! in.load() : press;
        engine.markModified();
        return true;
    }
    if (parts[1] == "mute") { setMuted (msg.isEmpty() ? ! isMuted() : press); return true; }

    if (parts[1] == "slot" && parts.size() >= 4)
    {
        auto slots = engine.getFlatSlots();
        auto n = parts[2].getIntValue();
        if (n < 1 || n > (int) slots.size()) return false;
        auto& slot = slots[(size_t) n - 1];

        if (parts[3] == "on")
        {
            engine.setBypassed (slot->uid, ! press);
            return true;
        }
        if (parts[3] == "bypass")
        {
            engine.setBypassed (slot->uid, press);
            return true;
        }
        if (parts[3] == "param" && parts.size() >= 5)
        {
            if (auto* p = getParam (slot->uid, parts[4].getIntValue() - 1))
                p->setValueNotifyingHost (juce::jlimit (0.0f, 1.0f, value));
            return true;
        }
    }

    if (parts[1] == "split" && parts.size() >= 4 && parts[3] == "mix")
    {
        auto splits = engine.getSplits();
        auto k = parts[2].getIntValue();
        if (k >= 1 && k <= (int) splits.size())
            engine.setSplitMix (splits[(size_t) k - 1]->uid, value);
        return true;
    }

    if (parts[1] == "master" && parts.size() >= 3)
    {
        if (parts[2] == "in")  engine.getInputStage()->gainDb  = normToDb (value);
        if (parts[2] == "out") engine.getOutputStage()->gainDb = normToDb (value);
        return true;
    }

    if (parts[1] == "bank" && parts.size() >= 3)
    {
        if (parts[2] == "next" && press && onBankNext) onBankNext();
        if (parts[2] == "prev" && press && onBankPrev) onBankPrev();
        if (parts[2] == "load" && ! msg.isEmpty())
        {
            if (msg[0].isString()) { if (onBankName) onBankName (msg[0].getString()); }
            else if (onBankIndex) onBankIndex ((int) value - 1, true);
        }
        if (parts[2] == "select" && ! msg.isEmpty() && onBankIndex)   // choose the bank without loading a preset
            onBankIndex ((int) value - 1, false);
        return true;
    }

    if (parts[1] == "preset" && parts.size() >= 3)
    {
        if (parts[2] == "next" && press && onPresetNext) onPresetNext();
        if (parts[2] == "prev" && press && onPresetPrev) onPresetPrev();
        if (parts[2] == "load" && ! msg.isEmpty())
        {
            if (msg[0].isString()) { if (onPresetName) onPresetName (msg[0].getString()); }
            else if (onPresetIndex) onPresetIndex ((int) value - 1);
        }
        return true;
    }

    return false;
}

//==============================================================================
void ControlSurface::handleIncomingMidiMessage (juce::MidiInput*, const juce::MidiMessage& m)
{
    if (m.isController() || m.isNoteOnOrOff() || m.isProgramChange())
    {
        juce::MessageManager::callAsync ([safe = juce::WeakReference<ControlSurface> (this), m]
        {
            if (auto* self = safe.get())
                self->handleMidi (m);
        });
    }
}

void ControlSurface::handleMidi (const juce::MidiMessage& m)
{
    lastMidiActivity = juce::Time::getMillisecondCounterHiRes();

    if (m.isProgramChange())
    {
        if (onPresetIndex) onPresetIndex (m.getProgramChangeNumber());
        return;
    }

    const bool isCC = m.isController();
    const int number = isCC ? m.getControllerNumber() : m.getNoteNumber();
    const float value = isCC ? (float) m.getControllerValue() / 127.0f
                             : (m.isNoteOn() ? 1.0f : 0.0f);

    if (learnState == LearnState::waitingForSource && (isCC || m.isNoteOn()))
    {
        finishLearn (isCC ? Mapping::Source::midiCC : Mapping::Source::midiNote, m.getChannel(), number, {});
        return;
    }

    // Standard MIDI bank select (CC 0) chooses the bank; the next Program Change loads from it.
    if (isCC && number == 0 && onBankIndex)
    {
        const bool mapped = std::any_of (mappings.begin(), mappings.end(), [] (const Mapping& mp)
        {
            return mp.source == Mapping::Source::midiCC && mp.number == 0;
        });
        if (! mapped)
        {
            onBankIndex (m.getControllerValue(), false);
            return;
        }
    }

    for (auto& map : mappings)
    {
        const bool srcMatch = (isCC && map.source == Mapping::Source::midiCC)
                           || (! isCC && map.source == Mapping::Source::midiNote);
        if (srcMatch && map.number == number && (map.midiChannel == 0 || map.midiChannel == m.getChannel()) && ! isOverridden (map))
            apply (map, value, value >= 0.5f);
    }
}

//==============================================================================
void ControlSurface::apply (Mapping& m, float value, bool isPress)
{
    const bool risingEdge = m.source == Mapping::Source::osc ? isPress : (isPress && ! m.lastPressed);
    m.lastPressed = isPress;

    switch (m.target)
    {
        case Mapping::Target::param:
            if (auto* p = getParam (m.slotUid, m.paramIndex))
            {
                if (m.isControllerButton() && ! m.toggle)
                {
                    p->setValueNotifyingHost (isPress ? 1.0f : 0.0f);       // momentary: on while held
                }
                else if (m.source == Mapping::Source::midiNote || m.isControllerButton())
                {
                    if (risingEdge) p->setValueNotifyingHost (p->getValue() >= 0.5f ? 0.0f : 1.0f);
                }
                else
                {
                    p->setValueNotifyingHost (value);
                }
            }
            break;

        case Mapping::Target::bypass:
            if (auto s = engine.findSlot (m.slotUid))
            {
                if (m.toggle) { if (risingEdge) engine.setBypassed (s->uid, ! s->bypassed); }
                else          engine.setBypassed (s->uid, ! isPress);
            }
            break;

        case Mapping::Target::splitMix:
            if (m.isControllerButton() && ! m.toggle)
                engine.setSplitMix (m.slotUid, isPress ? 1.0f : 0.0f);          // B only while held
            else if (m.isControllerButton())
            {
                if (risingEdge)
                    if (auto* item = engine.findItem (m.slotUid))
                        engine.setSplitMix (m.slotUid, item->mix >= 0.5f ? 0.0f : 1.0f);   // switch flips A <-> B
            }
            else
                engine.setSplitMix (m.slotUid, value);
            break;
        case Mapping::Target::tuner:
            if (m.toggle) { if (risingEdge) setTunerMode (! tunerMode); }
            else          setTunerMode (isPress);
            break;
        case Mapping::Target::stompSlot:
        {
            auto slots = engine.getFlatSlots();
            if (juce::isPositiveAndNotGreaterThan (m.paramIndex, (int) slots.size()) && m.paramIndex > 0)
            {
                auto& s = slots[(size_t) m.paramIndex - 1];
                if (m.toggle) { if (risingEdge) engine.setBypassed (s->uid, ! s->bypassed); }
                else          engine.setBypassed (s->uid, ! isPress);
            }
            break;
        }
        case Mapping::Target::presetSelect:
            if (risingEdge && onPresetIndex && m.paramIndex > 0) onPresetIndex (m.paramIndex - 1);
            break;
        case Mapping::Target::tapTempo:
            if (risingEdge) { engine.transport.tap(); if (onTempoTapped) onTempoTapped(); }
            break;
        case Mapping::Target::tempo:      // a pedal / fader: 40 - 240 BPM
            engine.transport.setBpm (40.0 + 200.0 * juce::jlimit (0.0f, 1.0f, value));
            break;
        case Mapping::Target::click:
            if (m.toggle) { if (risingEdge) engine.transport.clickOn = ! engine.transport.clickOn.load(); }
            else          engine.transport.clickOn = isPress;
            break;
        case Mapping::Target::instrumentInput:
        {
            auto& in = engine.getRack().instrumentInput;
            if (m.toggle) { if (risingEdge) in = ! in.load(); }
            else          in = isPress;
            engine.markModified();
            break;
        }
        case Mapping::Target::inputGain:  engine.getInputStage()->gainDb = normToDb (value); break;
        case Mapping::Target::outputGain: engine.getOutputStage()->gainDb = normToDb (value); break;
        case Mapping::Target::presetNext: if (risingEdge && onPresetNext) onPresetNext(); break;
        case Mapping::Target::presetPrev: if (risingEdge && onPresetPrev) onPresetPrev(); break;
        case Mapping::Target::bankNext:   if (risingEdge && onBankNext) onBankNext(); break;
        case Mapping::Target::bankPrev:   if (risingEdge && onBankPrev) onBankPrev(); break;
        case Mapping::Target::mute:
            if (m.toggle) { if (risingEdge) setMuted (! isMuted()); }
            else          setMuted (isPress);
            break;
    }
}

//==============================================================================
void ControlSurface::armLearn()
{
    pending = {};
    pendingHasControllerSource = false;
    learnCandidate = -1;
    learnArmedTime = juce::Time::getMillisecondCounterHiRes();
    learnState = LearnState::waitingForTarget;
    sendChangeMessage();
}

void ControlSurface::armLearnFor (Mapping::Target target, int uid)
{
    pending = {};
    pendingHasControllerSource = false;
    pending.target = target;
    pending.slotUid = uid;
    learnState = LearnState::waitingForSource;
    sendChangeMessage();
}

void ControlSurface::cancelLearn()
{
    pendingHasControllerSource = false;
    learnState = LearnState::off;
    sendChangeMessage();
}

void ControlSurface::finishLearn (Mapping::Source src, int channel, int number, const juce::String& address)
{
    Mapping m = pending;
    m.source = src;
    m.midiChannel = channel;
    m.number = number;
    m.oscAddress = address;
    // Momentary MIDI buttons usually want toggle; continuous controls are irrelevant.
    m.toggle = true;
    m.perPreset = m.isRackMapping();    // plugin controls go with the preset, host functions everywhere

    if (src == Mapping::Source::controller)
    {
        learnState = LearnState::off;
        setControllerMapping (m);
        return;
    }

    learnState = LearnState::off;
    addMapping (m);
}

void ControlSurface::addMapping (Mapping m)
{
    if (! m.perPreset) bindPlugin (m);

    // Replace an identical source -> target mapping rather than duplicating.
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(), [&] (const Mapping& x)
    {
        return x.source == m.source && x.number == m.number && x.oscAddress == m.oscAddress
            && x.target == m.target && x.slotUid == m.slotUid && x.paramIndex == m.paramIndex
            && x.perPreset == m.perPreset;
    }), mappings.end());

    mappings.push_back (m);
    sendChangeMessage();
}

bool ControlSurface::isOverridden (const Mapping& m) const
{
    if (m.isController())       // controller controls are one scope or the other: the other scope's leftovers do nothing
        return m.perPreset != isControllerPerPreset (m.ctrlUnit, m.ctrlControl, m.ctrlLayer);
    if (m.perPreset) return false;
    for (auto& x : mappings)
        if (x.perPreset && x.sameSource (m))
            return true;
    return false;
}

void ControlSurface::setMappingScope (int index, bool perPreset)
{
    if (! juce::isPositiveAndBelow (index, (int) mappings.size())) return;
    Mapping m = mappings[(size_t) index];
    if (m.perPreset == perPreset) return;
    if (! perPreset && m.isRackMapping() && m.slotUid == 0) return;    // nothing to follow
    if (m.isController()) { setControllerScope (m.ctrlUnit, m.ctrlControl, m.ctrlLayer, perPreset); return; }
    mappings.erase (mappings.begin() + index);
    m.perPreset = perPreset;
    if (m.isController()) setControllerMapping (m);
    else                  addMapping (m);
}

void ControlSurface::bindPlugin (Mapping& m) const
{
    if (m.target == Mapping::Target::splitMix)
    {
        m.pluginName = "split";
        m.pluginOccurrence = juce::jmax (0, engine.getSplitNumber (m.slotUid) - 1);
        return;
    }
    if (m.target != Mapping::Target::param && m.target != Mapping::Target::bypass) return;
    auto slot = engine.findSlot (m.slotUid);
    if (slot == nullptr) return;
    m.pluginName = slot->description.name;
    m.pluginOccurrence = 0;
    for (auto& s : engine.getFlatSlots())
    {
        if (s->uid == m.slotUid) break;
        if (s->description.name == m.pluginName) ++m.pluginOccurrence;
    }
}

void ControlSurface::resolveGlobalMappings()
{
    auto slots = engine.getFlatSlots();
    auto splits = engine.getSplits();
    bool changed = false;

    for (auto& m : mappings)
    {
        if (m.perPreset || ! m.isRackMapping() || m.pluginName.isEmpty()) continue;
        int uid = 0;
        if (m.target == Mapping::Target::splitMix)
        {
            if (juce::isPositiveAndBelow (m.pluginOccurrence, (int) splits.size()))
                uid = splits[(size_t) m.pluginOccurrence]->uid;
        }
        else
        {
            int n = 0;
            for (auto& s : slots)
                if (s->description.name == m.pluginName && n++ == m.pluginOccurrence) { uid = s->uid; break; }
        }
        if (uid != m.slotUid) { m.slotUid = uid; changed = true; }
    }
    if (changed) sendChangeMessage();
}

//==============================================================================
bool ControlSurface::isControllerPerPreset (const juce::String& unit, int control, int layer) const
{
    if (auto it = ctrlScopes.find (scopeKey (unit, control, layer)); it != ctrlScopes.end()) return it->second;
    // not chosen yet (older settings): whatever it already has; new controls are GLOBAL
    bool anyPreset = false;
    for (auto& m : mappings)
        if (m.sameControl (unit, control, layer))
        {
            if (! m.perPreset) return false;
            anyPreset = true;
        }
    return anyPreset;
}

void ControlSurface::setControllerMapping (const Mapping& mapping)
{
    Mapping m = mapping;
    m.perPreset = isControllerPerPreset (m.ctrlUnit, m.ctrlControl, m.ctrlLayer);
    ctrlScopes[scopeKey (m.ctrlUnit, m.ctrlControl, m.ctrlLayer)] = m.perPreset;
    if (! m.perPreset) bindPlugin (m);
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(), [&] (const Mapping& x)
    {
        return x.sameControl (m.ctrlUnit, m.ctrlControl, m.ctrlLayer) && x.perPreset == m.perPreset && x.sameFunction (m);
    }), mappings.end());
    mappings.push_back (m);
    if (m.perPreset) engine.markModified();
    if (onControllerMappingAdded) onControllerMappingAdded (m);
    sendChangeMessage();
}

void ControlSurface::removeControllerMapping (const Mapping& which)
{
    const Mapping w = which;     // may point into the vector
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(), [&] (const Mapping& x)
    {
        return x.sameControl (w.ctrlUnit, w.ctrlControl, w.ctrlLayer) && x.perPreset == w.perPreset && x.sameFunction (w);
    }), mappings.end());
    if (w.perPreset) engine.markModified();
    sendChangeMessage();
}

std::vector<const Mapping*> ControlSurface::findControllerMappings (const juce::String& unit, int control, int layer) const
{
    const bool perPreset = isControllerPerPreset (unit, control, layer);
    std::vector<const Mapping*> out;
    for (auto& m : mappings)
        if (m.sameControl (unit, control, layer) && m.perPreset == perPreset)
            out.push_back (&m);
    return out;
}

void ControlSurface::clearControllerMapping (const juce::String& unit, int control, int layer, int scope)
{
    if (scope < 0) scope = isControllerPerPreset (unit, control, layer) ? 1 : 0;
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(), [&] (const Mapping& x)
    {
        return x.sameControl (unit, control, layer) && x.perPreset == (scope == 1);
    }), mappings.end());
    sendChangeMessage();
}

const Mapping* ControlSurface::findControllerMapping (const juce::String& unit, int control, int layer, int scope) const
{
    const bool perPreset = scope < 0 ? isControllerPerPreset (unit, control, layer) : scope == 1;
    for (auto& m : mappings)
        if (m.sameControl (unit, control, layer) && m.perPreset == perPreset)
            return &m;
    return nullptr;
}

void ControlSurface::setControllerScope (const juce::String& unit, int control, int layer, bool perPreset)
{
    const bool was = isControllerPerPreset (unit, control, layer);
    std::vector<Mapping> moving;
    for (auto* m : findControllerMappings (unit, control, layer)) moving.push_back (*m);

    ctrlScopes[scopeKey (unit, control, layer)] = perPreset;
    if (onGlobalSettingsChanged) onGlobalSettingsChanged();      // keep the choice even if PEQUOD isn't closed normally
    if (was == perPreset) { sendChangeMessage(); return; }

    // The current functions carry across; nothing is left behind in the other scope.
    clearControllerMapping (unit, control, layer, 0);
    clearControllerMapping (unit, control, layer, 1);
    for (auto m : moving)
    {
        m.perPreset = perPreset;
        setControllerMapping (m);
    }
    sendChangeMessage();
}

void ControlSurface::setControllerLedColour (const juce::String& unit, int control, int layer, juce::uint32 colour)
{
    auto set = findControllerMappings (unit, control, layer);
    for (auto& m : mappings)
        if (std::find (set.begin(), set.end(), &m) != set.end())
            m.ledColour = colour;
    sendChangeMessage();
}

void ControlSurface::armLearnForController (const juce::String& unit, int control, int layer, const juce::String& label)
{
    pending = {};
    pending.source = Mapping::Source::controller;
    pending.ctrlUnit = unit;
    pending.ctrlControl = control;
    pending.ctrlLayer = layer;
    pendingHasControllerSource = true;
    pendingControllerLabel = label;
    learnCandidate = -1;
    learnArmedTime = juce::Time::getMillisecondCounterHiRes();
    learnState = LearnState::waitingForTarget;
    sendChangeMessage();
}

void ControlSurface::setTunerMode (bool on)
{
    tunerMode = on;
    setMuted (on);
}

bool ControlSurface::handleControllerEvent (const juce::String& unit, int control, int layer, float value, int delta)
{
    lastControllerActivity = juce::Time::getMillisecondCounterHiRes();
    const bool press = value >= 0.5f;

    // Learning with a fixed target (e.g. "+ HOST > Preset NEXT", or after touching a plugin knob): this is the source.
    if (learnState == LearnState::waitingForSource && ! pendingHasControllerSource && (press || delta != 0 || control == Mapping::ctrlPedal))
    {
        Mapping m = pending;
        m.source = Mapping::Source::controller;
        m.ctrlUnit = unit;
        m.ctrlControl = control;
        m.ctrlLayer = layer;
        m.toggle = true;
        m.perPreset = m.isRackMapping();
        m.ledColour = Mapping::ledEffectColour;
        learnState = LearnState::off;
        setControllerMapping (m);
        return true;
    }

    bool hit = false;
    for (auto& m : mappings)
    {
        if (! m.sameControl (unit, control, layer) || isOverridden (m)) continue;
        hit = true;

        if (delta != 0)   // encoder: relative changes
        {
            switch (m.target)
            {
                case Mapping::Target::param:
                    if (auto* p = getParam (m.slotUid, m.paramIndex))
                        p->setValueNotifyingHost (juce::jlimit (0.0f, 1.0f, p->getValue() + 0.01f * (float) delta));
                    break;
                case Mapping::Target::presetNext: case Mapping::Target::presetPrev:
                    if (delta > 0 ? (bool) onPresetNext : (bool) onPresetPrev) (delta > 0 ? onPresetNext : onPresetPrev)();
                    break;
                case Mapping::Target::bankNext: case Mapping::Target::bankPrev:
                    if (delta > 0 ? (bool) onBankNext : (bool) onBankPrev) (delta > 0 ? onBankNext : onBankPrev)();
                    break;
                case Mapping::Target::splitMix:
                    if (auto* item = engine.findItem (m.slotUid)) engine.setSplitMix (m.slotUid, item->mix + 0.02f * (float) delta);
                    break;
                case Mapping::Target::inputGain:
                    engine.getInputStage()->gainDb = juce::jlimit (-36.0f, 12.0f, engine.getInputStage()->gainDb.load() + 0.5f * (float) delta);
                    break;
                case Mapping::Target::outputGain:
                    engine.getOutputStage()->gainDb = juce::jlimit (-36.0f, 12.0f, engine.getOutputStage()->gainDb.load() + 0.5f * (float) delta);
                    break;
                case Mapping::Target::tempo:
                    engine.transport.setBpm (engine.transport.bpm.load() + (double) delta);
                    break;
                default:
                    apply (m, 1.0f, true); apply (m, 0.0f, false);
                    break;
            }
            continue;
        }

        apply (m, value, press);
    }
    return hit;
}

juce::String ControlSurface::getLearnStatusText() const
{
    switch (learnState.load())
    {
        case LearnState::waitingForTarget:
            return pendingHasControllerSource ? "LEARN: move a control on a plugin for " + pendingControllerLabel + "..."
                                              : juce::String ("LEARN: move a control on a plugin...");
        case LearnState::waitingForSource: return "LEARN: " + describeTarget (pending) + "  <-  press a footswitch, move a MIDI control or send OSC...";
        case LearnState::off: default:     return {};
    }
}

//==============================================================================
void ControlSurface::removeMapping (int index)
{
    if (juce::isPositiveAndBelow (index, (int) mappings.size()))
    {
        mappings.erase (mappings.begin() + index);
        sendChangeMessage();
    }
}

void ControlSurface::toggleMappingMode (int index)
{
    if (juce::isPositiveAndBelow (index, (int) mappings.size()))
    {
        mappings[(size_t) index].toggle = ! mappings[(size_t) index].toggle;
        sendChangeMessage();
    }
}

void ControlSurface::clearRackMappings()
{
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(),
                                    [] (const Mapping& m) { return m.perPreset; }),
                    mappings.end());
    sendChangeMessage();
}

void ControlSurface::slotRemoved (int slotUid)
{
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(), [slotUid] (const Mapping& m)
    {
        return m.perPreset && (m.target == Mapping::Target::param || m.target == Mapping::Target::bypass) && m.slotUid == slotUid;
    }), mappings.end());
    for (auto& m : mappings)      // global ones wait for that plugin to come back
        if (! m.perPreset && (m.target == Mapping::Target::param || m.target == Mapping::Target::bypass) && m.slotUid == slotUid)
            m.slotUid = 0;
    sendChangeMessage();
}

juce::String ControlSurface::describeTarget (const Mapping& m) const
{
    switch (m.target)
    {
        case Mapping::Target::param:
        {
            auto s = engine.findSlot (m.slotUid);
            auto* p = getParam (m.slotUid, m.paramIndex);
            return (s ? s->description.name : juce::String ("?")) + " / " + (p ? p->getName (32) : juce::String ("?"));
        }
        case Mapping::Target::bypass:
        {
            auto s = engine.findSlot (m.slotUid);
            return (s ? s->description.name : juce::String ("?")) + " / ON-OFF";
        }
        case Mapping::Target::splitMix:   return "Split " + juce::String (engine.getSplitNumber (m.slotUid)) + " / MIX";
        case Mapping::Target::inputGain:  return "Master IN";
        case Mapping::Target::outputGain: return "Master OUT";
        case Mapping::Target::presetNext: return "Preset NEXT";
        case Mapping::Target::presetPrev: return "Preset PREV";
        case Mapping::Target::mute:       return "MUTE";
        case Mapping::Target::tuner:      return "TUNER";
        case Mapping::Target::bankNext:   return "Bank NEXT";
        case Mapping::Target::bankPrev:   return "Bank PREV";
        case Mapping::Target::stompSlot:
        {
            auto slots = engine.getFlatSlots();
            juce::String now = juce::isPositiveAndNotGreaterThan (m.paramIndex, (int) slots.size()) && m.paramIndex > 0
                                 ? " (" + slots[(size_t) m.paramIndex - 1]->description.name + ")" : juce::String();
            return "Effect " + juce::String (m.paramIndex) + " ON-OFF" + now;
        }
        case Mapping::Target::presetSelect: return "Preset " + juce::String (m.paramIndex);
        case Mapping::Target::tapTempo:     return "TAP TEMPO";
        case Mapping::Target::tempo:        return "TEMPO";
        case Mapping::Target::click:        return "CLICK on/off";
        case Mapping::Target::instrumentInput: return "INPUT guitar / instrument";
    }
    return {};
}

juce::String ControlSurface::describeSource (const Mapping& m) const
{
    juce::String ch = m.midiChannel == 0 ? "" : " ch" + juce::String (m.midiChannel);
    switch (m.source)
    {
        case Mapping::Source::midiCC:   return "CC " + juce::String (m.number) + ch;
        case Mapping::Source::midiNote: return "Note " + juce::MidiMessage::getMidiNoteName (m.number, true, true, 3) + ch;
        case Mapping::Source::osc:      return m.oscAddress;
        case Mapping::Source::controller:
            return controllerSourceName ? controllerSourceName (m) : juce::String ("Controller");
    }
    return {};
}

//==============================================================================
static juce::XmlElement* mappingToXml (const Mapping& m, int targetRef)
{
    auto* x = new juce::XmlElement ("MAPPING");
    x->setAttribute ("target", targetNames[(int) m.target]);
    x->setAttribute ("ref", targetRef);
    x->setAttribute ("param", m.paramIndex);
    x->setAttribute ("source", sourceNames[(int) m.source]);
    x->setAttribute ("ch", m.midiChannel);
    x->setAttribute ("num", m.number);
    x->setAttribute ("osc", m.oscAddress);
    x->setAttribute ("toggle", m.toggle);
    if (m.isController())
    {
        x->setAttribute ("cu", m.ctrlUnit);
        x->setAttribute ("cc", m.ctrlControl);
        x->setAttribute ("cl", m.ctrlLayer);
    }
    if (m.ledColour != 0)
        x->setAttribute ("led", juce::String::toHexString ((int) m.ledColour));
    x->setAttribute ("scope", m.perPreset ? "preset" : "global");
    if (! m.perPreset && m.pluginName.isNotEmpty())
    {
        x->setAttribute ("plugin", m.pluginName);
        x->setAttribute ("nth", m.pluginOccurrence);
    }
    return x;
}

static Mapping mappingFromXml (const juce::XmlElement& x)
{
    Mapping m;
    m.target = enumFromName (x.getStringAttribute ("target"), targetNames, 17, Mapping::Target::param);
    m.paramIndex = x.getIntAttribute ("param", -1);
    m.source = enumFromName (x.getStringAttribute ("source"), sourceNames, 4, Mapping::Source::midiCC);
    m.midiChannel = x.getIntAttribute ("ch");
    m.number = x.getIntAttribute ("num");
    m.oscAddress = x.getStringAttribute ("osc");
    m.toggle = x.getBoolAttribute ("toggle", true);
    m.ctrlUnit = x.getStringAttribute ("cu");
    m.ctrlControl = x.getIntAttribute ("cc");
    m.ctrlLayer = x.getIntAttribute ("cl");
    m.ledColour = (juce::uint32) x.getStringAttribute ("led").getHexValue32();
    m.pluginName = x.getStringAttribute ("plugin");
    m.pluginOccurrence = x.getIntAttribute ("nth");
    return m;
}

std::unique_ptr<juce::XmlElement> ControlSurface::createRackMappingsXml() const
{
    auto root = std::make_unique<juce::XmlElement> ("MAPPINGS");
    for (auto& [key, v] : presetControllerActions)
    {
        auto parts = juce::StringArray::fromTokens (key, ":", {});
        if (parts.size() == 3 && isControllerPerPreset (parts[0], parts[1].getIntValue(), parts[2].getIntValue()))
        {
            auto* x = root->createNewChildElement ("CTRLACTION");
            x->setAttribute ("key", key);
            x->setAttribute ("json", juce::JSON::toString (v, true));
        }
    }
    for (auto& m : mappings)
    {
        if (! m.perPreset) continue;
        if (! m.isRackMapping())                                   // a host function just for this preset
        {
            root->addChildElement (mappingToXml (m, 0));
            continue;
        }
        const bool isSplit = m.target == Mapping::Target::splitMix;
        int ref = isSplit ? engine.getSplitNumber (m.slotUid) : engine.getFlatIndex (m.slotUid);
        if (ref > 0)
        {
            auto* x = mappingToXml (m, ref);                       // "ref" kept for older versions
            x->setAttribute ("sid", engine.getStableId (m.slotUid, isSplit));
            root->addChildElement (x);
        }
    }
    return root;
}

void ControlSurface::loadRackMappingsXml (const juce::XmlElement* rackRoot)
{
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(),
                                    [] (const Mapping& m) { return m.perPreset; }),
                    mappings.end());
    presetControllerActions.clear();

    if (rackRoot != nullptr)
        if (auto* xm = rackRoot->getChildByName ("MAPPINGS"))
            for (auto* x : xm->getChildWithTagNameIterator ("CTRLACTION"))
                presetControllerActions[x->getStringAttribute ("key")] = juce::JSON::parse (x->getStringAttribute ("json"));

    if (rackRoot != nullptr)
        if (auto* xm = rackRoot->getChildByName ("MAPPINGS"))
        {
            auto slots = engine.getFlatSlots();
            auto splits = engine.getSplits();

            for (auto* x : xm->getChildWithTagNameIterator ("MAPPING"))
            {
                auto m = mappingFromXml (*x);
                m.perPreset = true;
                if (m.isController())
                {
                    const auto key = scopeKey (m.ctrlUnit, m.ctrlControl, m.ctrlLayer);
                    if (ctrlScopes.count (key) == 0)
                    {
                        // Set up before PEQUOD remembered each control's choice: functions saved with a preset
                        // (and none global) mean it was a THIS PRESET control. Remember that from now on.
                        const bool hasGlobal = std::any_of (mappings.begin(), mappings.end(), [&] (const Mapping& g)
                        {
                            return ! g.perPreset && g.sameControl (m.ctrlUnit, m.ctrlControl, m.ctrlLayer);
                        });
                        ctrlScopes[key] = ! hasGlobal;
                        if (onGlobalSettingsChanged) onGlobalSettingsChanged();
                    }
                    if (! ctrlScopes[key]) continue;       // the control is GLOBAL: this preset's old functions don't apply
                }
                const bool isSplit = m.target == Mapping::Target::splitMix;
                if (! m.isRackMapping()) { mappings.push_back (m); continue; }

                // Newer presets: follow the module's saved ID wherever it sits in the rack.
                // If that module didn't load, the mapping is dropped rather than landing elsewhere.
                if (x->hasAttribute ("sid"))
                {
                    m.slotUid = engine.findUidByStableId (x->getStringAttribute ("sid"), isSplit);
                    if (m.slotUid != 0)
                        mappings.push_back (m);
                    continue;
                }

                auto ref = x->getIntAttribute ("ref");
                if (isSplit)
                {
                    if (ref < 1 || ref > (int) splits.size()) continue;
                    m.slotUid = splits[(size_t) ref - 1]->uid;
                }
                else
                {
                    if (ref < 1 || ref > (int) slots.size()) continue;
                    m.slotUid = slots[(size_t) ref - 1]->uid;
                }
                mappings.push_back (m);
            }
        }

    resolveGlobalMappings();
    if (onPresetMappingsLoaded) onPresetMappingsLoaded();
    sendChangeMessage();
}

std::unique_ptr<juce::XmlElement> ControlSurface::createGlobalXml() const
{
    auto root = std::make_unique<juce::XmlElement> ("CONTROL");
    root->setAttribute ("oscIn", oscInPort);
    root->setAttribute ("oscOutHost", oscOutHost);
    root->setAttribute ("oscOutPort", oscOutPort);
    for (auto& m : mappings)
        if (! m.perPreset)
            root->addChildElement (mappingToXml (m, m.target == Mapping::Target::splitMix ? m.pluginOccurrence + 1 : 0));
    for (auto& [key, perPreset] : ctrlScopes)
    {
        auto* x = root->createNewChildElement ("CTRLSCOPE");
        x->setAttribute ("key", key);
        x->setAttribute ("perPreset", perPreset);
    }
    return root;
}

void ControlSurface::loadGlobalXml (const juce::XmlElement* x)
{
    if (x != nullptr)
    {
        oscInPort  = x->getIntAttribute ("oscIn", 9000);
        oscOutHost = x->getStringAttribute ("oscOutHost", "127.0.0.1");
        oscOutPort = x->getIntAttribute ("oscOutPort", 9001);

        for (auto* xm : x->getChildWithTagNameIterator ("MAPPING"))
        {
            auto m = mappingFromXml (*xm);
            m.perPreset = false;
            if (! m.isRackMapping() || m.pluginName.isNotEmpty())
                mappings.push_back (m);
        }
    }

    if (x != nullptr)
        for (auto* sc : x->getChildWithTagNameIterator ("CTRLSCOPE"))
            ctrlScopes[sc->getStringAttribute ("key")] = sc->getBoolAttribute ("perPreset");
    resolveGlobalMappings();
    setOscInputPort (oscInPort);
    setOscOutput (oscOutHost, oscOutPort);
}

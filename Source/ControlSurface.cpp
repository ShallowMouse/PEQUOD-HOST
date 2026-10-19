#include "ControlSurface.h"
#include <algorithm>
#include <map>

namespace
{
    const char* targetNames[] = { "param", "bypass", "splitMix", "inputGain", "outputGain", "presetNext", "presetPrev", "mute" };
    const char* sourceNames[] = { "cc", "note", "osc" };

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
            pending = {};
            pending.target = Mapping::Target::param;
            pending.slotUid = (int) (c >> 32);
            pending.paramIndex = (int) (c & 0xffffffff);
            if (getParam (pending.slotUid, pending.paramIndex) != nullptr)
            {
                learnState = LearnState::waitingForSource;
                sendChangeMessage();
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
        if (m.source == Mapping::Source::osc && m.oscAddress == address)
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

    for (auto& map : mappings)
    {
        const bool srcMatch = (isCC && map.source == Mapping::Source::midiCC)
                           || (! isCC && map.source == Mapping::Source::midiNote);
        if (srcMatch && map.number == number && (map.midiChannel == 0 || map.midiChannel == m.getChannel()))
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
                if (m.source == Mapping::Source::midiNote)
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

        case Mapping::Target::splitMix:   engine.setSplitMix (m.slotUid, value); break;
        case Mapping::Target::inputGain:  engine.getInputStage()->gainDb = normToDb (value); break;
        case Mapping::Target::outputGain: engine.getOutputStage()->gainDb = normToDb (value); break;
        case Mapping::Target::presetNext: if (risingEdge && onPresetNext) onPresetNext(); break;
        case Mapping::Target::presetPrev: if (risingEdge && onPresetPrev) onPresetPrev(); break;
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
    learnCandidate = -1;
    learnArmedTime = juce::Time::getMillisecondCounterHiRes();
    learnState = LearnState::waitingForTarget;
    sendChangeMessage();
}

void ControlSurface::armLearnFor (Mapping::Target target, int uid)
{
    pending = {};
    pending.target = target;
    pending.slotUid = uid;
    learnState = LearnState::waitingForSource;
    sendChangeMessage();
}

void ControlSurface::cancelLearn()
{
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

    // Replace an identical source -> target mapping rather than duplicating.
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(), [&] (const Mapping& x)
    {
        return x.source == m.source && x.number == m.number && x.oscAddress == m.oscAddress
            && x.target == m.target && x.slotUid == m.slotUid && x.paramIndex == m.paramIndex;
    }), mappings.end());

    mappings.push_back (m);
    learnState = LearnState::off;
    sendChangeMessage();
}

juce::String ControlSurface::getLearnStatusText() const
{
    switch (learnState.load())
    {
        case LearnState::waitingForTarget: return "LEARN: move a control on a plugin...";
        case LearnState::waitingForSource: return "LEARN: " + describeTarget (pending) + "  <-  move a MIDI control or send OSC...";
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
                                    [] (const Mapping& m) { return m.isRackMapping(); }),
                    mappings.end());
    sendChangeMessage();
}

void ControlSurface::slotRemoved (int slotUid)
{
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(), [slotUid] (const Mapping& m)
    {
        return (m.target == Mapping::Target::param || m.target == Mapping::Target::bypass) && m.slotUid == slotUid;
    }), mappings.end());
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
    return x;
}

static Mapping mappingFromXml (const juce::XmlElement& x)
{
    Mapping m;
    m.target = enumFromName (x.getStringAttribute ("target"), targetNames, 8, Mapping::Target::param);
    m.paramIndex = x.getIntAttribute ("param", -1);
    m.source = enumFromName (x.getStringAttribute ("source"), sourceNames, 3, Mapping::Source::midiCC);
    m.midiChannel = x.getIntAttribute ("ch");
    m.number = x.getIntAttribute ("num");
    m.oscAddress = x.getStringAttribute ("osc");
    m.toggle = x.getBoolAttribute ("toggle", true);
    return m;
}

std::unique_ptr<juce::XmlElement> ControlSurface::createRackMappingsXml() const
{
    auto root = std::make_unique<juce::XmlElement> ("MAPPINGS");
    for (auto& m : mappings)
    {
        if (! m.isRackMapping()) continue;
        int ref = m.target == Mapping::Target::splitMix ? engine.getSplitNumber (m.slotUid)
                                                        : engine.getFlatIndex (m.slotUid);
        if (ref > 0)
            root->addChildElement (mappingToXml (m, ref));
    }
    return root;
}

void ControlSurface::loadRackMappingsXml (const juce::XmlElement* rackRoot)
{
    mappings.erase (std::remove_if (mappings.begin(), mappings.end(),
                                    [] (const Mapping& m) { return m.isRackMapping(); }),
                    mappings.end());

    if (rackRoot != nullptr)
        if (auto* xm = rackRoot->getChildByName ("MAPPINGS"))
        {
            auto slots = engine.getFlatSlots();
            auto splits = engine.getSplits();

            for (auto* x : xm->getChildWithTagNameIterator ("MAPPING"))
            {
                auto m = mappingFromXml (*x);
                auto ref = x->getIntAttribute ("ref");
                if (m.target == Mapping::Target::splitMix)
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

    sendChangeMessage();
}

std::unique_ptr<juce::XmlElement> ControlSurface::createGlobalXml() const
{
    auto root = std::make_unique<juce::XmlElement> ("CONTROL");
    root->setAttribute ("oscIn", oscInPort);
    root->setAttribute ("oscOutHost", oscOutHost);
    root->setAttribute ("oscOutPort", oscOutPort);
    for (auto& m : mappings)
        if (! m.isRackMapping())
            root->addChildElement (mappingToXml (m, 0));
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
            if (! m.isRackMapping())
                mappings.push_back (m);
        }
    }

    setOscInputPort (oscInPort);
    setOscOutput (oscOutHost, oscOutPort);
}

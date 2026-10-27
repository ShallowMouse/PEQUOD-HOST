#include "PequodNative.h"
#include "PequodPanel.h"

namespace PequodNative
{
//==============================================================================
const std::vector<Info>& all()
{
    static const std::vector<Info> list = []
    {
        std::vector<Info> v;
        const int n = pq_count();
        v.reserve ((size_t) juce::jmax (0, n));
        for (int i = 0; i < n; ++i)
        {
            auto* s = pq_info (i);
            if (s == nullptr) continue;
            auto j = juce::JSON::parse (juce::String::fromUTF8 (s));
            Info inf;
            inf.index      = i;
            inf.name       = j["name"].toString();
            inf.vendor     = j["vendor"].toString();
            inf.classId    = j["id"].toString();
            inf.categories = j["categories"].toString();
            inf.instrument = (bool) j["instrument"];
            inf.meters     = (int) j["meters"];
            if (auto* ps = j["params"].getArray())
                for (auto& p : *ps)
                    inf.params.push_back ({ p["id"].toString(), p["name"].toString(), p["unit"].toString(),
                                            (float) (double) p["default"], (int) p["steps"] });
            v.push_back (std::move (inf));
        }
        return v;
    }();
    return list;
}

const Info* find (const juce::String& id)
{
    if (! id.startsWith (idPrefix)) return nullptr;
    const auto cid = id.substring ((int) std::strlen (idPrefix));
    for (auto& i : all())
        if (i.classId == cid)
            return &i;
    return nullptr;
}

juce::PluginDescription describe (const Info& i)
{
    juce::PluginDescription d;
    d.name = i.name;
    d.descriptiveName = i.name;
    d.pluginFormatName = formatName;
    d.category = i.categories;
    d.manufacturerName = i.vendor;
    d.version = "built-in";
    d.fileOrIdentifier = idPrefix + i.classId;
    d.uniqueId = d.deprecatedUid = (int) i.classId.hashCode();
    d.isInstrument = i.instrument;
    d.numInputChannels = i.instrument ? 0 : 2;
    d.numOutputChannels = 2;
    d.hasSharedContainer = false;
    d.hasARAExtension = false;
    return d;
}

const juce::var& panelOf (const Info& i)
{
    // parsed on first use (all on the message thread)
    auto& p = const_cast<Info&> (i).panel;
    if (p.isVoid())
        if (auto* s = pq_panel (i.index))
            p = juce::JSON::parse (juce::String::fromUTF8 (s));
    return p;
}

void registerAll (juce::KnownPluginList& list)
{
    // drop entries for engine plugins that no longer exist, then add (or refresh) every one
    for (auto& t : list.getTypes())
        if (t.pluginFormatName == formatName && find (t.fileOrIdentifier) == nullptr)
            list.removeType (t);

    for (auto& i : all())
    {
        auto d = describe (i);
        bool same = false;
        for (auto& t : list.getTypes())
            if (t.fileOrIdentifier == d.fileOrIdentifier)
                same = t.name == d.name && t.manufacturerName == d.manufacturerName && t.category == d.category;
        if (! same)
        {
            for (auto& t : list.getTypes())
                if (t.fileOrIdentifier == d.fileOrIdentifier)
                    list.removeType (t);
            list.addType (d);
        }
    }
}

//==============================================================================
void Format::findAllTypesForFile (juce::OwnedArray<juce::PluginDescription>& results, const juce::String& id)
{
    if (auto* i = find (id))
        results.add (new juce::PluginDescription (describe (*i)));
}

juce::String Format::getNameOfPluginFromIdentifier (const juce::String& id)
{
    if (auto* i = find (id)) return i->name;
    return id;
}

juce::StringArray Format::searchPathsForPlugins (const juce::FileSearchPath&, bool, bool)
{
    juce::StringArray ids;
    for (auto& i : all())
        ids.add (idPrefix + i.classId);
    return ids;
}

void Format::createPluginInstance (const juce::PluginDescription& d, double, int, PluginCreationCallback callback)
{
    if (auto* i = find (d.fileOrIdentifier))
        callback (std::make_unique<Instance> (*i), {});
    else
        callback (nullptr, "This PEQUOD plugin isn't in this version of PEQUOD Host: " + d.name);
}

//==============================================================================
namespace
{
    class Param : public juce::HostedAudioProcessorParameter
    {
    public:
        Param (const Instance& o, int i) : owner (o), index (i), p (o.info().params[(size_t) i]) {}

        float getValue() const override { return pq_get_param (owner.handle(), index); }
        void setValue (float v) override { pq_set_param (owner.handle(), index, v); }
        float getDefaultValue() const override { return p.defaultValue; }
        juce::String getName (int maxLen) const override { return p.name.substring (0, maxLen); }
        juce::String getLabel() const override { return p.unit; }
        juce::String getParameterID() const override { return p.id; }
        int getNumSteps() const override { return p.steps > 0 ? p.steps + 1 : AudioProcessor::getDefaultNumParameterSteps(); }
        bool isDiscrete() const override { return p.steps > 0; }
        bool isBoolean() const override { return p.steps == 1; }

        juce::String getText (float v, int maxLen) const override
        {
            char buf[128] = {};
            pq_param_text (owner.handle(), index, v, buf, (int32_t) sizeof (buf));
            return juce::String::fromUTF8 (buf).substring (0, maxLen);
        }

        float getValueForText (const juce::String& text) const override
        {
            float out = getValue();
            pq_param_from_text (owner.handle(), index, text.toRawUTF8(), &out);
            return out;
        }

    private:
        const Instance& owner;
        const int index;
        const Info::Param p;
    };

    juce::AudioProcessor::BusesProperties busesFor (const Info& i)
    {
        juce::AudioProcessor::BusesProperties b;
        if (! i.instrument)
            b = b.withInput ("Input", juce::AudioChannelSet::stereo(), true);
        return b.withOutput ("Output", juce::AudioChannelSet::stereo(), true);
    }
}

Instance::Instance (const Info& i)
    : juce::AudioPluginInstance (busesFor (i)), inf (i), h (pq_create (i.index))
{
    for (int p = 0; p < (int) inf.params.size(); ++p)
        addHostedParameter (std::make_unique<Param> (*this, p));
}

Instance::~Instance()
{
    pq_destroy (h);
}

bool Instance::isBusesLayoutSupported (const BusesLayout& l) const
{
    const auto out = l.getMainOutputChannelSet();
    if (out != juce::AudioChannelSet::stereo() && out != juce::AudioChannelSet::mono()) return false;
    if (inf.instrument) return l.getMainInputChannelSet().isDisabled();
    return l.getMainInputChannelSet() == out;
}

void Instance::prepareToPlay (double sampleRate, int blockSize)
{
    const int ch = juce::jmax (1, getTotalNumOutputChannels());
    setLatencySamples ((int) pq_prepare (h, sampleRate, ch, blockSize));
    scratch.setSize (2, juce::jmax (1, blockSize));
    ptrs.assign (2, nullptr);
}

void Instance::processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi)
{
    const int n = buffer.getNumSamples();
    const int nch = juce::jmin (buffer.getNumChannels(), 2);
    if (n <= 0 || nch <= 0) return;

    if (inf.instrument)
        buffer.clear();

    for (const auto m : midi)
    {
        const auto& msg = m.getMessage();
        const int at = juce::jlimit (0, n - 1, m.samplePosition);
        if (msg.isNoteOn())                         pq_midi (h, at, 0, msg.getNoteNumber(), msg.getFloatVelocity());
        else if (msg.isNoteOff())                   pq_midi (h, at, 1, msg.getNoteNumber(), 0.0f);
        else if (msg.isAllNotesOff() || msg.isAllSoundOff()) pq_midi (h, at, 4, 0, 0.0f);
        else if (msg.isController())                pq_midi (h, at, 2, msg.getControllerNumber(), (float) msg.getControllerValue() / 127.0f);
        else if (msg.isPitchWheel())                pq_midi (h, at, 3, 0, (float) (msg.getPitchWheelValue() - 8192) / 8192.0f);
    }

    double tempo = 120.0, beats = 0.0;
    int flags = 0;
    if (auto* ph = getPlayHead())
        if (auto pos = ph->getPosition())
        {
            if (auto b = pos->getBpm())         { tempo = *b; flags |= 1; }
            if (auto q = pos->getPpqPosition()) { beats = *q; flags |= 2; }
            if (pos->getIsPlaying())             flags |= 4;
        }

    ptrs.resize ((size_t) nch);
    for (int c = 0; c < nch; ++c)
        ptrs[(size_t) c] = buffer.getWritePointer (c);
    pq_process (h, ptrs.data(), nch, n, tempo, beats, flags);
}

juce::AudioProcessorEditor* Instance::createEditor()
{
    return new PequodPanel (*this);
}

void Instance::getStateInformation (juce::MemoryBlock& mb)
{
    const int len = pq_get_state (h, nullptr, 0);
    juce::HeapBlock<char> buf ((size_t) len + 1, true);
    pq_get_state (h, buf.get(), len + 1);
    mb.replaceAll (buf.get(), (size_t) len);
}

void Instance::setStateInformation (const void* data, int size)
{
    juce::MemoryBlock state (data, (size_t) size);

    // A rack saved with the VST3 version: JUCE wraps the plugin's own state in XML.
    if (auto xml = getXmlFromBinary (data, size))
        if (xml->hasTagName ("VST3PluginState"))
            if (auto* comp = xml->getChildByName ("IComponent"))
            {
                juce::MemoryBlock inner;
                if (inner.fromBase64Encoding (comp->getAllSubText()))
                    state = inner;
            }

    if (pq_set_state (h, (const uint8_t*) state.getData(), (int32_t) state.getSize()) != 0)
        for (auto* p : getParameters())
            p->sendValueChangedMessageToListeners (p->getValue());
}
}

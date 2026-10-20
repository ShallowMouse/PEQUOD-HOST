#pragma once

#include <JuceHeader.h>

//==============================================================================
/** What the controller says about itself ({"t":"info"}). */
struct ControllerUnit
{
    int slot = -1, type = 0, layer = 0;
    juce::String kind, name, uid;   // kind: footswitch | pedal | pedal_switch | encoder | button | knob

    bool hasButtons() const  { return kind == "footswitch" || kind == "button"; }
    int  numButtons() const  { return hasButtons() ? 4 : (kind == "pedal_switch" ? 1 : 0); }
    bool hasPedal() const    { return kind == "pedal" || kind == "pedal_switch"; }
    bool isEncoder() const   { return kind == "encoder"; }
};

struct ControllerInfo
{
    juce::String id, name, fw, via;
    int proto = 0, layers = 1, layer = 0, brightness = 255;
    bool globalLayers = false;
    bool wifiConnected = false, wifiAp = false;
    juce::String ssid, ssid2, ip;
    bool otaOk = false;
    int64_t otaMax = 0;
    std::vector<ControllerUnit> units;

    const ControllerUnit* findUnit (const juce::String& uid) const
    {
        for (auto& u : units) if (u.uid == uid) return &u;
        return nullptr;
    }
    const ControllerUnit* findSlot (int slot) const
    {
        for (auto& u : units) if (u.slot == slot) return &u;
        return nullptr;
    }

    static ControllerInfo fromVar (const juce::var&);
};

//==============================================================================
/** PEQUOD Link framing: JSON split into 7-bit frames for SysEx (USB) or UDP. */
namespace LinkFraming
{
    constexpr int sysExPayload = 100;
    constexpr int udpPayload   = 1000;
    constexpr int udpPort      = 7400;

    /** Non-ASCII characters become \uXXXX so every byte is 7-bit. */
    juce::String toAsciiJson (const juce::var&);

    std::vector<juce::MemoryBlock> encode (const juce::String& asciiJson, uint8_t msgId, bool forSysEx);

    /** Joins frames (starting at the 'J' byte) back into a message. */
    struct Assembler
    {
        int id = -1, next = 0, count = 0;
        juce::MemoryOutputStream text;
        /** Returns true when a whole message is ready in `out`. */
        bool feed (const uint8_t* frame, int len, juce::String& out);
    };
}

//==============================================================================
/** Finds the foot controller (USB first, then Wi-Fi) and keeps the connection alive.
    All callbacks happen on the message thread; send() may be called from any thread. */
class ControllerLink : private juce::MidiInputCallback,
                       private juce::Timer,
                       private juce::Thread
{
public:
    explicit ControllerLink (juce::AudioDeviceManager&);
    ~ControllerLink() override;

    enum class Via { none, usb, wifi };

    bool isConnected() const noexcept           { return via != Via::none; }
    Via getVia() const noexcept                 { return via; }
    const ControllerInfo& getInfo() const       { return info; }
    juce::String describeConnection() const;

    /** Sends a JSON object to the controller. Thread-safe. */
    bool send (const juce::var& message);

    /** Optional fixed address for Wi-Fi (when broadcasts are blocked). */
    void setManualAddress (const juce::String& ip) { manualIp = ip.trim(); }
    juce::String getManualAddress() const          { return manualIp; }

    struct Listener
    {
        virtual ~Listener() = default;
        virtual void controllerConnected() {}
        virtual void controllerDisconnected() {}
        virtual void controllerInfoChanged() {}
        virtual void controllerMessage (const juce::String& type, const juce::var& message) = 0;
    };
    void addListener (Listener* l)    { listeners.add (l); }
    void removeListener (Listener* l) { listeners.remove (l); }

private:
    juce::AudioDeviceManager& deviceManager;
    juce::ListenerList<Listener> listeners;

    Via via = Via::none;
    ControllerInfo info;
    double lastRx = 0, lastPing = 0, lastDiscover = 0, lastUsbScan = 0;
    std::atomic<uint8_t> msgId { 0 };

    // USB
    struct Probe { juce::String inputId, outputId; double sentAt = 0; bool failed = false; };
    std::vector<Probe> probes;
    juce::StringArray lastDeviceSignature;
    std::unique_ptr<juce::MidiOutput> usbOut;
    juce::String usbInputId;
    std::map<juce::String, LinkFraming::Assembler> usbAssemblers;   // per MIDI input

    // Wi-Fi
    std::unique_ptr<juce::DatagramSocket> socket;
    juce::String wifiIp, manualIp;
    LinkFraming::Assembler udpAssembler;

    juce::CriticalSection sendLock;

    void timerCallback() override;
    void run() override;
    void handleIncomingMidiMessage (juce::MidiInput*, const juce::MidiMessage&) override;

    void scanUsb();
    void discoverWifi();
    void handleMessage (const juce::String& json, Via from, const juce::String& source);
    void connectVia (Via, const juce::String& source, const ControllerInfo&);
    void disconnect();
    bool sendTo (const juce::var&, Via, const juce::String& target);
    bool sendUsbRaw (juce::MidiOutput&, const juce::String& json);
    bool sendUdpRaw (const juce::String& ip, const juce::String& json);

    JUCE_DECLARE_NON_COPYABLE (ControllerLink)
};

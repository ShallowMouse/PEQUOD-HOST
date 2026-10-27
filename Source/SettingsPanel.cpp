#include "SettingsPanel.h"

namespace
{
    void styleCaption (juce::Label& l, const juce::String& text)
    {
        l.setText (text, juce::dontSendNotification);
        l.setFont (Pequod::heading (12.0f));
        l.setColour (juce::Label::textColourId, Pequod::Colours::mustard);
    }

    void styleLabel (juce::Label& l, const juce::String& text)
    {
        l.setText (text, juce::dontSendNotification);
        l.setFont (Pequod::heading (11.0f, false));
        l.setColour (juce::Label::textColourId, Pequod::Colours::textDim);
    }

    //==========================================================================
    class AudioPage : public juce::Component, private juce::Timer
    {
    public:
        explicit AudioPage (HostEngine& e)
            : engine (e),
              selector (e.deviceManager, 0, 2, 0, 2, true, false, true, false)
        {
            styleCaption (inputCaption, "GUITAR INPUT");
            styleLabel (inputHelp, "Which interface input your guitar is plugged into. It is sent to both sides of the rack.");
            inputMode.addItemList ({ "Stereo (both inputs)", "Left - input 1", "Right - input 2", "Left + Right mixed" }, 1);
            inputMode.setSelectedItemIndex (engine.getInputStage()->inputMode.load(), juce::dontSendNotification);
            inputMode.onChange = [this] { engine.getInputStage()->inputMode = inputMode.getSelectedItemIndex(); };

            styleCaption (deviceCaption, "AUDIO DEVICE & MIDI INPUTS");

            for (auto* c : std::initializer_list<juce::Component*> { &inputCaption, &inputHelp, &inputMode, &deviceCaption, &selector })
                addAndMakeVisible (c);
            startTimerHz (4);
        }

        void resized() override
        {
            auto r = getLocalBounds().reduced (16, 12);
            inputCaption.setBounds (r.removeFromTop (22));
            auto row = r.removeFromTop (28);
            inputMode.setBounds (row.removeFromLeft (240));
            inputHelp.setBounds (r.removeFromTop (22));
            r.removeFromTop (12);
            deviceCaption.setBounds (r.removeFromTop (22));
            selector.setBounds (r);
        }

    private:
        HostEngine& engine;
        juce::Label inputCaption, inputHelp, deviceCaption;
        juce::ComboBox inputMode;
        juce::AudioDeviceSelectorComponent selector;

        void timerCallback() override
        {
            if (inputMode.getSelectedItemIndex() != engine.getInputStage()->inputMode.load())
                inputMode.setSelectedItemIndex (engine.getInputStage()->inputMode.load(), juce::dontSendNotification);
        }
    };

    //==========================================================================
    class OscPage : public juce::Component, private juce::Timer, private juce::ChangeListener
    {
    public:
        explicit OscPage (ControlSurface& c) : control (c)
        {
            styleCaption (receiveCaption, "RECEIVE");
            styleCaption (sendCaption, "SEND FEEDBACK TO");
            styleLabel (portLabel, "PORT");
            styleLabel (hostLabel, "HOST / IP");
            styleLabel (outPortLabel, "PORT");

            for (auto* ed : { &inPort, &outHost, &outPort })
            {
                ed->setFont (Pequod::heading (14.0f, false));
                ed->onReturnKey = [this] { apply(); };
                ed->onFocusLost = [this] { apply(); };
            }
            inPort.setInputRestrictions (5, "0123456789");
            outPort.setInputRestrictions (5, "0123456789");

            syncButton.setTooltip ("Send every slot name, on/off state and parameter value to the controller");
            syncButton.onClick = [this] { control.sendFullState(); };

            help.setMultiLine (true);
            help.setReadOnly (true);
            help.setCaretVisible (false);
            help.setFont (Pequod::heading (12.0f, false));
            help.setColour (juce::TextEditor::backgroundColourId, Pequod::Colours::well);
            help.setColour (juce::TextEditor::textColourId, Pequod::Colours::textDim);
            help.setText ("/pequod/slot/{n}/param/{p}   f 0-1   set a parameter (slot and parameter from 1)\n"
                          "/pequod/slot/{n}/on          0/1     slot on / off\n"
                          "/pequod/split/{k}/mix        f 0-1   split A/B fader\n"
                          "/pequod/master/in  /out      f 0-1   levels (0.75 = 0 dB)\n"
                          "/pequod/mute                 0/1     output mute (no argument = toggle)\n"
                          "/pequod/preset/next  /prev           step presets in the bank\n"
                          "/pequod/preset/load          i or s  preset number in the bank, or name\n"
                          "/pequod/bank/next  /prev             step banks\n"
                          "/pequod/bank/load            i or s  bank number or name (loads a preset)\n"
                          "/pequod/bank/select          i       choose a bank without loading\n"
                          "/pequod/sync                         send everything back\n\n"
                          "Feedback includes names, on/off states, parameter values, tuner\n"
                          "(/pequod/tuner/note, cents, hz), mute, bank and preset names.\n"
                          "Any other address can be mapped with LEARN.", false);

            for (auto* comp : std::initializer_list<juce::Component*> { &receiveCaption, &sendCaption, &portLabel, &hostLabel, &outPortLabel,
                                                                      &inPort, &outHost, &outPort, &syncButton, &help })
                addAndMakeVisible (comp);

            refreshFields();
            control.addChangeListener (this);
            startTimerHz (15);
        }

        ~OscPage() override { control.removeChangeListener (this); }

        void paint (juce::Graphics& g) override
        {
            auto led = [&] (juce::Rectangle<int> r, bool on, bool flash, const juce::String& text)
            {
                auto c = r.removeFromLeft (14).toFloat().withSizeKeepingCentre (9, 9);
                g.setColour (flash ? Pequod::Colours::text : on ? Pequod::Colours::mustard : Pequod::Colours::danger);
                g.fillEllipse (c);
                g.setColour (Pequod::Colours::textDim);
                g.setFont (Pequod::heading (11.0f, false));
                g.drawText (text, r.withTrimmedLeft (4), juce::Justification::centredLeft);
            };
            const bool recentIn = juce::Time::getMillisecondCounterHiRes() - control.lastOscActivity < 120.0;
            led (inLed, control.isOscInConnected(), recentIn, control.isOscInConnected() ? "LISTENING" : "PORT IN USE");
            led (outLed, control.isOscOutConnected(), false, control.isOscOutConnected() ? "READY" : "NOT CONNECTED");
        }

        void resized() override
        {
            auto r = getLocalBounds().reduced (16, 12);
            receiveCaption.setBounds (r.removeFromTop (22));
            auto row = r.removeFromTop (28);
            portLabel.setBounds (row.removeFromLeft (80));
            inPort.setBounds (row.removeFromLeft (90));
            row.removeFromLeft (12);
            inLed = row.removeFromLeft (160);

            r.removeFromTop (14);
            sendCaption.setBounds (r.removeFromTop (22));
            row = r.removeFromTop (28);
            hostLabel.setBounds (row.removeFromLeft (80));
            outHost.setBounds (row.removeFromLeft (180));
            row.removeFromLeft (12);
            outPortLabel.setBounds (row.removeFromLeft (44));
            outPort.setBounds (row.removeFromLeft (90));
            row.removeFromLeft (12);
            outLed = row.removeFromLeft (160);

            r.removeFromTop (14);
            syncButton.setBounds (r.removeFromTop (28).removeFromLeft (160));
            r.removeFromTop (14);
            help.setBounds (r);
        }

    private:
        ControlSurface& control;
        juce::Label receiveCaption, sendCaption, portLabel, hostLabel, outPortLabel;
        juce::TextEditor inPort, outHost, outPort, help;
        juce::TextButton syncButton { "SEND SYNC" };
        juce::Rectangle<int> inLed, outLed;

        void refreshFields()
        {
            if (! inPort.hasKeyboardFocus (false))  inPort.setText (juce::String (control.getOscInputPort()), false);
            if (! outHost.hasKeyboardFocus (false)) outHost.setText (control.getOscOutHost(), false);
            if (! outPort.hasKeyboardFocus (false)) outPort.setText (juce::String (control.getOscOutPort()), false);
        }

        void apply()
        {
            auto in = inPort.getText().getIntValue();
            auto host = outHost.getText().trim();
            auto out = outPort.getText().getIntValue();
            if (in != control.getOscInputPort() || ! control.isOscInConnected())
                control.setOscInputPort (in);
            if (host != control.getOscOutHost() || out != control.getOscOutPort() || ! control.isOscOutConnected())
                control.setOscOutput (host, out);
            repaint();
        }

        void changeListenerCallback (juce::ChangeBroadcaster*) override { refreshFields(); repaint(); }
        void timerCallback() override { repaint (inLed.expanded (4)); }
    };

    //==========================================================================
    /** Network output to Hawthorne on another PC (Hawthorne Link). */
    class NetworkPage : public juce::Component, private juce::Timer
    {
    public:
        explicit NetworkPage (HostEngine& e) : engine (e)
        {
            styleCaption (caption, "SEND TO HAWTHORNE OVER THE NETWORK");
            styleLabel (nameLabel, "NAME");

            onToggle.setButtonText ("Send PEQUOD's output to Hawthorne (Hawthorne Link)");
            dryToggle.setButtonText ("Also send the dry guitar on channels 3-4 (to re-amp later)");
            for (auto* t : { &onToggle, &dryToggle })
            {
                t->setColour (juce::ToggleButton::tickColourId, Pequod::Colours::mustard);
                t->setColour (juce::ToggleButton::textColourId, Pequod::Colours::text);
                t->onClick = [this] { apply(); };
                addAndMakeVisible (t);
            }
            onToggle.setToggleState (engine.isNetworkOutputOn(), juce::dontSendNotification);
            dryToggle.setToggleState (engine.isNetworkSendingDry(), juce::dontSendNotification);

            nameEditor.setFont (Pequod::heading (14.0f, false));
            nameEditor.setText (engine.networkOut.getName(), false);
            nameEditor.setTooltip ("How PEQUOD appears in Hawthorne's NETWORK window");
            nameEditor.onReturnKey = [this] { apply(); };
            nameEditor.onFocusLost = [this] { apply(); };
            addAndMakeVisible (nameEditor);

            for (auto* comp : std::initializer_list<juce::Component*> { &caption, &nameLabel })
                addAndMakeVisible (comp);
            startTimerHz (4);
        }

        void paint (juce::Graphics& g) override
        {
            auto r = statusArea;
            const bool on = engine.isNetworkOutputOn();
            auto subs = engine.networkOut.getSubscribers();
            const double sr = engine.getRack().getCurrentSampleRate();

            juce::String line1;
            juce::Colour c = Pequod::Colours::textDim;
            if (! on)                { line1 = "OFF"; }
            else if (! error.isEmpty()) { line1 = error; c = Pequod::Colours::danger; }
            else if (subs.empty())   { line1 = "READY - waiting for Hawthorne to connect"; c = Pequod::Colours::mustard; }
            else                     { line1 = "SENDING " + juce::String (engine.isNetworkSendingDry() ? 4 : 2) + " ch to "
                                               + juce::String ((int) subs.size()) + (subs.size() == 1 ? " receiver" : " receivers"); c = Pequod::Colours::mustard; }
            g.setFont (Pequod::heading (13.0f));
            g.setColour (c);
            g.drawText ("> " + line1, r.removeFromTop (22), juce::Justification::centredLeft);

            g.setFont (Pequod::heading (12.0f, false));
            g.setColour (Pequod::Colours::textDim);
            if (on)
            {
                juce::StringArray to;
                for (auto& s : subs) to.add (s.address);
                g.drawText ("> " + juce::String (sr / 1000.0, 1) + " kHz   24-bit   port " + juce::String (engine.networkOut.getControlPort())
                            + (to.isEmpty() ? juce::String() : "   ->  " + to.joinIntoString (", ")),
                            r.removeFromTop (20), juce::Justification::centredLeft, true);
                const auto drops = engine.networkOut.getQueueOverruns();
                g.setColour (drops > 0 ? Pequod::Colours::danger : Pequod::Colours::textDim);
                g.drawText ("> packets " + juce::String (engine.networkOut.getPacketsSent())
                            + (drops > 0 ? "   !! the network couldn't keep up: " + juce::String (drops) + " packets dropped" : juce::String()),
                            r.removeFromTop (20), juce::Justification::centredLeft, true);
            }

            r.removeFromTop (12);
            g.setFont (Pequod::body (13.0f));
            g.setColour (Pequod::Colours::textDim);
            g.drawFittedText ("In Hawthorne, open NETWORK and pick \"" + engine.networkOut.getName() + "\": it shows up as a block of inputs "
                              "(1-2 = what PEQUOD plays, after OUT, the limiter and MUTE; 3-4 = the dry guitar if that's on). "
                              "Run both PCs at the same sample rate. For bit-perfect transfer, clock this PC's interface from Hawthorne's "
                              "with word clock; otherwise Hawthorne corrects the drift. If Hawthorne is on another subnet or VLAN, "
                              "add this PC's IP address in Hawthorne's NETWORK window.",
                              r.removeFromTop (110), juce::Justification::topLeft, 7);
        }

        void resized() override
        {
            auto r = getLocalBounds().reduced (16, 12);
            caption.setBounds (r.removeFromTop (22));
            r.removeFromTop (4);
            onToggle.setBounds (r.removeFromTop (28));
            auto row = r.removeFromTop (28);
            nameLabel.setBounds (row.removeFromLeft (80));
            nameEditor.setBounds (row.removeFromLeft (300));
            r.removeFromTop (6);
            dryToggle.setBounds (r.removeFromTop (28));
            r.removeFromTop (14);
            statusArea = r;
        }

    private:
        HostEngine& engine;
        juce::Label caption, nameLabel;
        juce::ToggleButton onToggle, dryToggle;
        juce::TextEditor nameEditor;
        juce::Rectangle<int> statusArea;
        juce::String error;

        void apply()
        {
            error.clear();
            if (! engine.setNetworkOutput (onToggle.getToggleState(), nameEditor.getText(), dryToggle.getToggleState(), error))
                onToggle.setToggleState (false, juce::dontSendNotification);
            nameEditor.setText (engine.networkOut.getName(), false);
            repaint();
        }

        void timerCallback() override { repaint (statusArea); }
    };
}

//==============================================================================
SettingsPanel::SettingsPanel (HostEngine& e, ControlSurface& c)
{
    tabs.setTabBarDepth (32);
    tabs.setOutline (0);
    tabs.setColour (juce::TabbedComponent::backgroundColourId, Pequod::Colours::panel);
    tabs.addTab ("AUDIO & MIDI", Pequod::Colours::panel, new AudioPage (e), true);
    tabs.addTab ("OSC", Pequod::Colours::panel, new OscPage (c), true);
    tabs.addTab ("NETWORK", Pequod::Colours::panel, new NetworkPage (e), true);
    addAndMakeVisible (tabs);
    setSize (620, 640);
}

SettingsPanel::~SettingsPanel() = default;

void SettingsPanel::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::panel);
}

void SettingsPanel::resized()
{
    tabs.setBounds (getLocalBounds());
}

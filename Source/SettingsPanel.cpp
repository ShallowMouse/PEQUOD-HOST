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
}

//==============================================================================
SettingsPanel::SettingsPanel (HostEngine& e, ControlSurface& c)
{
    tabs.setTabBarDepth (32);
    tabs.setOutline (0);
    tabs.setColour (juce::TabbedComponent::backgroundColourId, Pequod::Colours::panel);
    tabs.addTab ("AUDIO & MIDI", Pequod::Colours::panel, new AudioPage (e), true);
    tabs.addTab ("OSC", Pequod::Colours::panel, new OscPage (c), true);
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

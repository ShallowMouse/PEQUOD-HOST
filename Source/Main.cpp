#include "MainComponent.h"

class PequodHostApplication : public juce::JUCEApplication
{
public:
    const juce::String getApplicationName() override    { return JUCE_APPLICATION_NAME_STRING; }
    const juce::String getApplicationVersion() override { return JUCE_APPLICATION_VERSION_STRING; }
    bool moreThanOneInstanceAllowed() override          { return false; }

    void initialise (const juce::String&) override
    {
        juce::LookAndFeel::setDefaultLookAndFeel (&lookAndFeel);
        mainWindow = std::make_unique<MainWindow> (getApplicationName());

        // Test hook: quit automatically after N ms (used for automated smoke tests).
        auto quitAfter = juce::SystemStats::getEnvironmentVariable ("PEQUOD_QUIT_AFTER_MS", {}).getIntValue();
        if (quitAfter > 0)
            juce::Timer::callAfterDelay (quitAfter, [] { juce::JUCEApplication::getInstance()->systemRequestedQuit(); });
    }

    void shutdown() override
    {
        mainWindow = nullptr;
        juce::LookAndFeel::setDefaultLookAndFeel (nullptr);
    }

    void systemRequestedQuit() override { quit(); }
    void anotherInstanceStarted (const juce::String&) override {}

    class MainWindow : public juce::DocumentWindow
    {
    public:
        explicit MainWindow (const juce::String& name)
            : DocumentWindow (name, Pequod::Colours::bg, DocumentWindow::allButtons)
        {
            setUsingNativeTitleBar (true);
            setContentOwned (new MainComponent(), true);
            setResizable (true, true);
            setResizeLimits (1300, 700, 8000, 6000);

            juce::Rectangle<int> area (0, 0, 1920, 1080);
            if (auto* d = juce::Desktop::getInstance().getDisplays().getPrimaryDisplay()) area = d->userBounds.toNearestInt();
            centreWithSize (juce::jmin (getWidth(), area.getWidth() - 40), juce::jmin (getHeight(), area.getHeight() - 40));
            setVisible (true);
            setFullScreen (true);          // with the Windows title bar this is "maximised", not full screen
            showOnTop();

            // Windows can hand a program started by an installer, a shortcut or the updater a "start minimised"
            // request; PEQUOD always opens as a normal window in front.
            juce::Component::SafePointer<MainWindow> safe (this);
            for (int ms : { 300, 1200 })
                juce::Timer::callAfterDelay (ms, [safe] { if (safe != nullptr && safe->isMinimised()) safe->showOnTop(); });
        }

        void showOnTop()
        {
            if (isMinimised()) setMinimised (false);
            if (! isFullScreen()) setFullScreen (true);
            toFront (true);
        }

        void closeButtonPressed() override { juce::JUCEApplication::getInstance()->systemRequestedQuit(); }
    };

private:
    PequodLookAndFeel lookAndFeel;
    std::unique_ptr<MainWindow> mainWindow;
};

START_JUCE_APPLICATION (PequodHostApplication)

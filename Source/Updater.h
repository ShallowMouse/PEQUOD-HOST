#pragma once

#include <JuceHeader.h>

//==============================================================================
/** Checks the public releases repo for a newer PEQUOD Host when it starts, and installs it:
    downloads PEQUOD-Host-Setup-x.y.z.exe, runs it (it closes PEQUOD, installs, starts it again). */
class Updater : private juce::Thread,
                private juce::AsyncUpdater
{
public:
    static constexpr const char* releasesRepo = "ShallowMouse/PEQUOD-Host-Releases";

    Updater();
    ~Updater() override;

    void checkSoon();                 // in the background, a few seconds after start

    bool isUpdateAvailable() const    { return available; }
    juce::String getLatestVersion() const { return latestVersion; }
    juce::String getReleaseNotes() const  { return notes; }
    bool isDownloading() const        { return downloading; }
    float getProgress() const         { return progress; }

    /** Downloads the installer and runs it; PEQUOD quits so it can be replaced. */
    void install();

    std::function<void()> onChange;                        // availability / progress changed (message thread)
    std::function<void (const juce::String&)> onStatus;

    static juce::String currentVersion();
    static juce::String changesSince (const juce::String& installed, const juce::String& latest, const juce::String& latestBody);
    static int compareVersions (const juce::String& a, const juce::String& b);   // <0 a older, 0 same, >0 a newer

private:
    std::atomic<bool> available { false }, downloading { false };
    std::atomic<float> progress { 0.0f };
    juce::String latestVersion, notes, installerUrl, installerName, error;
    juce::CriticalSection lock;
    enum class Job { none, check, download } job = Job::none;
    juce::File downloaded;

    void run() override;
    void handleAsyncUpdate() override;
    void doCheck();
    void doDownload();
};

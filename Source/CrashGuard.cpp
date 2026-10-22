#include "CrashGuard.h"

#if JUCE_WINDOWS
 #ifndef NOMINMAX
  #define NOMINMAX
 #endif
 #ifndef WIN32_LEAN_AND_MEAN
  #define WIN32_LEAN_AND_MEAN
 #endif
 #include <windows.h>
#endif

namespace
{
    thread_local const char* tlPlugin = nullptr;
    thread_local const char* tlDoing = nullptr;

    juce::File runningFile, reportFile;

   #if JUCE_WINDOWS
    wchar_t reportPath[1024] = {};

    void writeRaw (HANDLE h, const char* s)
    {
        if (s == nullptr) return;
        DWORD written = 0;
        WriteFile (h, s, (DWORD) strlen (s), &written, nullptr);
    }

    LONG WINAPI onCrash (EXCEPTION_POINTERS*)
    {
        // Runs on the crashing thread, so the thread-local names say what it was doing.
        // Only plain Win32 calls in here: the heap may be broken.
        if (reportPath[0] != 0)
        {
            HANDLE h = CreateFileW (reportPath, GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
            if (h != INVALID_HANDLE_VALUE)
            {
                writeRaw (h, tlPlugin != nullptr ? tlPlugin : "");
                writeRaw (h, "\n");
                writeRaw (h, tlDoing != nullptr ? tlDoing : "");
                writeRaw (h, "\n");
                CloseHandle (h);
            }
        }
        return EXCEPTION_CONTINUE_SEARCH;
    }
   #endif
}

namespace CrashGuard
{
    LastRun start (const juce::File& folder)
    {
        folder.createDirectory();
        runningFile = folder.getChildFile ("PEQUOD running.txt");
        reportFile  = folder.getChildFile ("PEQUOD crash.txt");

        LastRun last;
        last.crashed = runningFile.existsAsFile();
        if (last.crashed && reportFile.existsAsFile())
        {
            auto lines = juce::StringArray::fromLines (reportFile.loadFileAsString());
            last.plugin = lines[0].trim();
            last.doing  = lines[1].trim();
        }
        reportFile.deleteFile();
        runningFile.replaceWithText (juce::Time::getCurrentTime().toISO8601 (true));

       #if JUCE_WINDOWS
        reportFile.getFullPathName().copyToUTF16 ((juce::CharPointer_UTF16::CharType*) reportPath, sizeof (reportPath) - 2);
        SetUnhandledExceptionFilter (onCrash);
       #endif
        return last;
    }

    void cleanExit()
    {
        runningFile.deleteFile();
        reportFile.deleteFile();
    }

    Scope::Scope (const char* plugin, const char* doing) noexcept
        : prevPlugin (tlPlugin), prevDoing (tlDoing)
    {
        tlPlugin = plugin;
        tlDoing = doing;
    }

    Scope::~Scope() noexcept
    {
        tlPlugin = prevPlugin;
        tlDoing = prevDoing;
    }
}

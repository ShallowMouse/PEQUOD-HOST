#include "WorkerPool.h"
#include <thread>

#if JUCE_WINDOWS
 #ifndef NOMINMAX
  #define NOMINMAX
 #endif
 #include <windows.h>
 #include <avrt.h>
 #if JUCE_MSVC
  #pragma comment (lib, "avrt.lib")
 #endif
#endif

#if JUCE_INTEL
 #include <immintrin.h>
#endif

namespace
{
    inline void cpuPause() noexcept
    {
       #if JUCE_INTEL
        _mm_pause();
       #elif JUCE_ARM
        __asm__ __volatile__ ("yield");
       #endif
    }
}

//==============================================================================
class WorkerPool::Worker : public juce::Thread
{
public:
    Worker (WorkerPool& p, int index) : Thread ("PEQUOD audio worker " + juce::String (index)), pool (p) {}

    ~Worker() override
    {
        signalThreadShouldExit();
        wake.signal();
        stopThread (2000);
    }

    void start()
    {
        if (! startRealtimeThread (RealtimeOptions{}.withPriority (10)))
            startThread (Priority::highest);
    }

    juce::WaitableEvent wake;

private:
    WorkerPool& pool;

    void run() override
    {
        promoteCurrentThreadForAudio();

        while (! threadShouldExit())
        {
            if (pool.runOnePending())
                continue;

            // Spin briefly: the next audio callback usually posts work again soon.
            bool found = false;
            const auto spinUntil = juce::Time::getMillisecondCounterHiRes() + 0.3;
            while (juce::Time::getMillisecondCounterHiRes() < spinUntil)
            {
                if (pool.runOnePending()) { found = true; break; }
                for (int i = 0; i < 32; ++i) cpuPause();
            }
            if (found) continue;

            pool.sleepers.fetch_add (1);
            if (! pool.runOnePending())      // re-check after announcing we're about to sleep
                wake.wait (5);
            pool.sleepers.fetch_sub (1);
        }
    }
};

//==============================================================================
WorkerPool::WorkerPool (int numThreads)
{
    for (int i = 0; i < numThreads; ++i)
        workers.push_back (std::make_unique<Worker> (*this, i + 1));
    for (auto& w : workers)
        w->start();
}

WorkerPool::~WorkerPool()
{
    workers.clear();
}

void WorkerPool::promoteCurrentThreadForAudio()
{
   #if JUCE_WINDOWS
    DWORD taskIndex = 0;
    AvSetMmThreadCharacteristicsW (L"Pro Audio", &taskIndex);
   #endif
}

bool WorkerPool::post (const Task& t, std::atomic<int>* remaining) noexcept
{
    for (auto& s : slots)
    {
        int expected = 0;
        if (s.state.compare_exchange_strong (expected, 1, std::memory_order_acquire))
        {
            s.task = t;
            s.remaining = remaining;
            s.state.store (2, std::memory_order_release);
            return true;
        }
    }
    return false;
}

bool WorkerPool::runOnePending() noexcept
{
    for (auto& s : slots)
    {
        if (s.state.load (std::memory_order_relaxed) != 2) continue;

        int expected = 2;
        if (s.state.compare_exchange_strong (expected, 3, std::memory_order_acquire))
        {
            const Task t = s.task;
            auto* remaining = s.remaining;
            s.state.store (0, std::memory_order_release);   // slot can be reused; we hold copies

            t.fn (t.arg);
            remaining->fetch_sub (1, std::memory_order_acq_rel);
            return true;
        }
    }
    return false;
}

void WorkerPool::wakeSleepers() noexcept
{
    if (sleepers.load (std::memory_order_acquire) > 0)
        for (auto& w : workers)
            w->wake.signal();
}

void WorkerPool::runParallel (Task* tasks, int numTasks) noexcept
{
    if (numTasks <= 0) return;

    if (numTasks == 1 || workers.empty())
    {
        for (int i = 0; i < numTasks; ++i)
            tasks[i].fn (tasks[i].arg);
        return;
    }

    std::atomic<int> remaining { 0 };

    for (int i = 1; i < numTasks; ++i)
    {
        remaining.fetch_add (1, std::memory_order_relaxed);
        if (! post (tasks[i], &remaining))
        {
            remaining.fetch_sub (1, std::memory_order_relaxed);
            tasks[i].fn (tasks[i].arg);           // no free slot: just do it here
        }
    }

    wakeSleepers();
    tasks[0].fn (tasks[0].arg);

    // Help out until all of our tasks are finished.
    while (remaining.load (std::memory_order_acquire) > 0)
    {
        if (! runOnePending())
            cpuPause();
    }
}

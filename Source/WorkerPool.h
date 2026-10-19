#pragma once

#include <JuceHeader.h>
#include <atomic>
#include <memory>
#include <vector>

//==============================================================================
/** A small pool of real-time worker threads for the audio callback.

    runParallel() hands tasks 1..n-1 to the workers, runs task 0 on the calling
    thread, then helps with any still-pending tasks until everything is done. No
    allocation or locking on the audio path (a sleeping worker is woken with an
    event, which is the only system call). Nested calls are safe because waiting
    threads execute pending work themselves instead of blocking. */
class WorkerPool
{
public:
    struct Task
    {
        void (*fn) (void*) = nullptr;
        void* arg = nullptr;
    };

    explicit WorkerPool (int numThreads);
    ~WorkerPool();

    int getNumThreads() const noexcept { return (int) workers.size(); }

    void runParallel (Task* tasks, int numTasks) noexcept;

    /** Called on each worker thread at start-up (used to join MMCSS "Pro Audio" on Windows). */
    static void promoteCurrentThreadForAudio();

private:
    static constexpr int numSlots = 64;

    struct Slot
    {
        std::atomic<int> state { 0 };       // 0 free, 1 claimed by poster, 2 pending, 3 running
        Task task;
        std::atomic<int>* remaining = nullptr;
    };

    class Worker;

    Slot slots[numSlots];
    std::vector<std::unique_ptr<Worker>> workers;
    std::atomic<int> sleepers { 0 };

    bool post (const Task&, std::atomic<int>* remaining) noexcept;
    bool runOnePending() noexcept;
    void wakeSleepers() noexcept;

    JUCE_DECLARE_NON_COPYABLE (WorkerPool)
};

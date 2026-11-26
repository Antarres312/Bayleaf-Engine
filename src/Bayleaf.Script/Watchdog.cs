using Bayleaf.Script.Sdk;

namespace Bayleaf.Script;

/// <summary>
/// The thread a play session's scripts run on, and the watchdog over it. A C# script runs in the
/// engine's process with no fuel to run out of, so the play step hands its scripts' work to this thread
/// and waits at most a time limit; a step that takes longer stops play mode. .NET cannot stop a thread
/// from outside, so a script stuck in an endless loop keeps that thread busy until the program ends:
/// the thread is left behind with the abandoned host and its scene, which nothing else uses, and the
/// console says so.
/// </summary>
internal sealed class ScriptThread : IDisposable
{
    /// <summary>The stack a script thread gets: deep enough for ordinary recursion (a script that recurses without end still ends the program, as in any .NET code).</summary>
    private const int StackBytes = 16 << 20;

    private readonly Thread thread;
    private readonly ManualResetEventSlim go = new(false);
    private readonly ManualResetEventSlim done = new(false);
    private readonly IWorld world;
    private Action? job;
    private volatile bool closing;

    public ScriptThread(IWorld world)
    {
        this.world = world;
        thread = new Thread(Loop, StackBytes) { IsBackground = true, Name = "Bayleaf scripts" };
        thread.Start();
    }

    /// <summary>The watchdog stopped a job: the thread may still be running it and takes no more.</summary>
    public bool Stalled { get; private set; }

    /// <summary>Runs <paramref name="work"/> on the script thread and waits for it, at most <paramref name="limit"/>; false when it took longer (the thread is then left to it).</summary>
    public bool Run(Action work, TimeSpan limit)
    {
        if (Stalled || closing)
        {
            return false;
        }
        job = work;
        done.Reset();
        go.Set();
        if (done.Wait(limit))
        {
            return true;
        }
        Stalled = true;
        return false;
    }

    private void Loop()
    {
        Runtime.World = world;
        while (true)
        {
            go.Wait();
            go.Reset();
            if (closing)
            {
                return;
            }
            try
            {
                job?.Invoke();
            }
            finally
            {
                done.Set();
            }
        }
    }

    public void Dispose()
    {
        if (closing)
        {
            return;
        }
        closing = true;
        if (Stalled)
        {
            // The stuck job keeps the thread; it is a background thread and ends with the program.
            return;
        }
        go.Set();
        thread.Join(TimeSpan.FromSeconds(1));
        go.Dispose();
        done.Dispose();
    }
}

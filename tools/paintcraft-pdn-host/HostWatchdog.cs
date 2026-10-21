using System.Diagnostics;
using System.IO;

// The Unix Wine launcher is not necessarily the parent of all Windows workers.
// A shared heartbeat bounds their lifetime even if the native editor is killed.
internal sealed class HostWatchdog : IDisposable
{
    private readonly string path;
    private readonly System.Threading.Timer timer;
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private long lastValue;
    private long lastChange;

    private HostWatchdog(string path)
    {
        this.path = Path.GetFullPath(path);
        lastValue = Read();
        timer = new System.Threading.Timer(_ => Check(), null, 250, 250);
    }

    internal static HostWatchdog? Start(string[] args)
    {
        int i = Array.IndexOf(args, "--watchdog");
        if (i < 0) return null;
        if (i + 1 == args.Length) throw new ArgumentException("Missing watchdog path");
        return new HostWatchdog(args[i + 1]);
    }

    internal static void Forward(string[] args, ProcessStartInfo child)
    {
        int i = Array.IndexOf(args, "--watchdog");
        if (i >= 0 && i + 1 < args.Length)
        {
            child.ArgumentList.Add("--watchdog");
            child.ArgumentList.Add(args[i + 1]);
        }
    }

    private long Read()
    {
        using var file = new FileStream(path, FileMode.Open, FileAccess.Read,
            FileShare.ReadWrite | FileShare.Delete);
        Span<byte> bytes = stackalloc byte[8];
        file.ReadExactly(bytes);
        return System.Buffers.Binary.BinaryPrimitives.ReadInt64LittleEndian(bytes);
    }

    private void Check()
    {
        lock (elapsed)
        {
            try
            {
                long value = Read();
                if (value != lastValue) { lastValue = value; lastChange = elapsed.ElapsedMilliseconds; }
                if (elapsed.ElapsedMilliseconds - lastChange < 5000) return;
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
            // No output is committed after losing the editor's heartbeat.
            Environment.Exit(2);
        }
    }

    public void Dispose() => timer.Dispose();
}

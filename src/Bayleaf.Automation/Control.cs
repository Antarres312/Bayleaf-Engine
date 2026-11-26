using System.Collections.Concurrent;
using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Automation;

/// <summary>What answers control requests: the editor, on its UI thread.</summary>
public interface IControlHandler
{
    /// <summary>The command list, as <c>engine.commands</c> returns it.</summary>
    JsonValue Commands();

    /// <summary>Runs one command for an agent, inside the agent's allowed folders.</summary>
    Result<JsonValue, string> Execute(string command, JsonValue parameters);
}

/// <summary>One request for the editor to answer on its UI thread.</summary>
public sealed class ControlRequest
{
    private readonly BlockingCollection<Result<JsonValue, string>> reply;

    internal ControlRequest(string method, JsonValue parameters, BlockingCollection<Result<JsonValue, string>> reply)
    {
        Method = method;
        Params = parameters;
        this.reply = reply;
    }

    public string Method { get; }

    public JsonValue Params { get; }

    public void Respond(Result<JsonValue, string> result)
    {
        // The connection may have closed while the editor worked; nothing to do then.
        reply.TryAdd(result);
    }
}

/// <summary>
/// Control channel of a running editor: newline-delimited JSON over a loopback TCP socket. Request
/// <c>{"id": 1, "method": "engine.execute", "params": {"command": "...", "params": {...}}}</c>, reply
/// <c>{"id": 1, "ok": true, "result": ...}</c> or <c>{"id": 1, "ok": false, "error": "..."}</c>. The
/// first request on every connection must be <c>{"method": "auth", "params": {"token": "..."}}</c>
/// with the token from the token file; anything else closes the connection. Methods besides
/// <c>auth</c>: <c>engine.commands</c> and <c>engine.execute</c>, answered by the editor's UI thread,
/// which drains <see cref="Pending"/> every frame.
/// </summary>
public sealed class ControlServer : IDisposable
{
    /// <summary>Longest request line.</summary>
    public const int MaxLine = 1 << 20;

    /// <summary>Open connections at most.</summary>
    public const int MaxConnections = 16;

    /// <summary>A connection that sends nothing for this long is closed.</summary>
    public static readonly TimeSpan IdleTimeout = TimeSpan.FromSeconds(300);

    /// <summary>Longest wait for the editor to answer one request.</summary>
    public static readonly TimeSpan ReplyTimeout = TimeSpan.FromSeconds(60);

    private readonly TcpListener listener;
    private readonly ConcurrentQueue<ControlRequest> requests = new();
    private int open;

    private ControlServer(TcpListener listener) => this.listener = listener;

    public IPEndPoint Address => (IPEndPoint)listener.LocalEndpoint;

    /// <summary>
    /// Listens on <c>127.0.0.1:port</c> (0 picks a free port). <paramref name="wake"/> is called when
    /// a request arrives, so an idle editor redraws and answers it.
    /// </summary>
    public static ControlServer Start(int port, string token, Action wake)
    {
        var listener = new TcpListener(IPAddress.Loopback, port);
        listener.Start();
        var server = new ControlServer(listener);
        new Thread(() => server.Accept(token, wake)) { IsBackground = true, Name = "bayleaf control" }.Start();
        return server;
    }

    public void Dispose() => listener.Stop();

    private void Accept(string token, Action wake)
    {
        while (true)
        {
            TcpClient client;
            try
            {
                client = listener.AcceptTcpClient();
            }
            catch (Exception e) when (e is SocketException or ObjectDisposedException or InvalidOperationException)
            {
                return;
            }
            if (Volatile.Read(ref open) >= MaxConnections)
            {
                using (client)
                {
                    WriteLine(client.GetStream(), Reply(JsonValue.Null, false, new JsonString("too many connections")));
                }
                continue;
            }
            Interlocked.Increment(ref open);
            new Thread(() =>
            {
                using (client)
                {
                    try
                    {
                        Serve(client, token, wake);
                    }
                    catch (Exception e) when (e is IOException or SocketException or ObjectDisposedException)
                    {
                        // The connection broke; the next one starts afresh.
                    }
                }
                Interlocked.Decrement(ref open);
            })
            { IsBackground = true, Name = "bayleaf control connection" }.Start();
        }
    }

    /// <summary>Requests waiting for an answer.</summary>
    public IEnumerable<ControlRequest> Pending()
    {
        while (requests.TryDequeue(out var request))
        {
            yield return request;
        }
    }

    /// <summary>Answers every waiting request with <paramref name="handler"/>; how many were answered.</summary>
    public int Answer(IControlHandler handler)
    {
        var waiting = Pending().ToList();
        foreach (var request in waiting)
        {
            Result<JsonValue, string> result = request.Method switch
            {
                "engine.commands" => handler.Commands(),
                "engine.execute" => request.Params.Get("command")?.AsString() is { } command
                    ? handler.Execute(command, request.Params.Get("params") ?? new JsonObject())
                    : Result.Fail("engine.execute needs params.command"),
                _ => Result.Fail($"unknown method {request.Method}; use engine.commands or engine.execute"),
            };
            request.Respond(result);
        }
        return waiting.Count;
    }

    private void Serve(TcpClient client, string token, Action wake)
    {
        client.ReceiveTimeout = (int)IdleTimeout.TotalMilliseconds;
        var stream = client.GetStream();
        var authenticated = false;
        while (ReadLine(stream) is { } line)
        {
            if (!JsonReader.Parse(line).TryGet(out var request, out var parseError))
            {
                WriteLine(stream, Reply(JsonValue.Null, false, new JsonString($"not JSON: {parseError}")));
                continue;
            }
            var id = request.Get("id") ?? JsonValue.Null;
            var method = request.Get("method")?.AsString() ?? "";
            var parameters = request.Get("params") ?? JsonValue.Null;
            if (!authenticated)
            {
                var given = parameters.Get("token")?.AsString() ?? "";
                if (method == "auth" && CryptographicOperations.FixedTimeEquals(Encoding.UTF8.GetBytes(given), Encoding.UTF8.GetBytes(token)))
                {
                    authenticated = true;
                    WriteLine(stream, Reply(id, true, new JsonObject()));
                    continue;
                }
                WriteLine(stream, Reply(id, false, new JsonString("authentication required")));
                return;
            }
            if (method == "auth")
            {
                WriteLine(stream, Reply(id, false, new JsonString("already authenticated")));
                continue;
            }
            using var answer = new BlockingCollection<Result<JsonValue, string>>(1);
            requests.Enqueue(new ControlRequest(method, parameters, answer));
            wake();
            var reply = answer.TryTake(out var result, ReplyTimeout)
                ? result.TryGet(out var value, out var error) ? Reply(id, true, value) : Reply(id, false, new JsonString(error))
                // The command may still run; the client should check before sending it again.
                : Reply(id, false, new JsonString($"timeout: no reply in {(int)ReplyTimeout.TotalSeconds}s"));
            WriteLine(stream, reply);
        }
    }

    private static string Reply(JsonValue id, bool ok, JsonValue payload) =>
        new JsonObject { { "id", id }, { "ok", JsonValue.From(ok) }, { ok ? "result" : "error", payload } }.Canonical().ToCompact();

    internal static void WriteLine(Stream stream, string line)
    {
        var bytes = Encoding.UTF8.GetBytes(line + "\n");
        stream.Write(bytes);
        stream.Flush();
    }

    /// <summary>One line of at most <see cref="MaxLine"/> bytes, without its newline; null at the end of the stream.</summary>
    internal static string? ReadLine(Stream stream)
    {
        var buffer = new List<byte>();
        while (true)
        {
            var b = stream.ReadByte();
            if (b < 0)
            {
                if (buffer.Count == 0)
                {
                    return null;
                }
                break;
            }
            if (b == '\n')
            {
                break;
            }
            buffer.Add((byte)b);
            if (buffer.Count > MaxLine)
            {
                throw new IOException("request line too long");
            }
        }
        while (buffer.Count > 0 && buffer[^1] == '\r')
        {
            buffer.RemoveAt(buffer.Count - 1);
        }
        try
        {
            return new UTF8Encoding(false, true).GetString([.. buffer]);
        }
        catch (DecoderFallbackException)
        {
            throw new IOException("request is not UTF-8");
        }
    }

    /// <summary>The token in <paramref name="path"/>, creating the file with a new random 256-bit token if it does not exist.</summary>
    public static Result<string, string> LoadOrCreateToken(string path)
    {
        try
        {
            if (File.Exists(path) && File.ReadAllText(path).Trim() is { Length: >= 32 } existing)
            {
                // A file from an older version may still be readable by others.
                var restricted = RestrictToOwner(path);
                return restricted.IsOk ? existing : restricted.AsFailure();
            }
            var token = Convert.ToHexStringLower(RandomNumberGenerator.GetBytes(32));
            if (Path.GetDirectoryName(path) is { Length: > 0 } folder)
            {
                Directory.CreateDirectory(folder);
            }
            File.WriteAllText(path, token);
            var limited = RestrictToOwner(path);
            return limited.IsOk ? token : limited.AsFailure();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return Result.Fail(e.Message);
        }
    }

    /// <summary>
    /// Leaves only the current user able to read the file: on Windows by dropping the rules it
    /// inherits and granting only the current user, with the system's <c>icacls</c> tool; elsewhere
    /// with owner-only file modes.
    /// </summary>
    private static Result<Unit, string> RestrictToOwner(string path)
    {
        if (!OperatingSystem.IsWindows())
        {
            File.SetUnixFileMode(path, UnixFileMode.UserRead | UnixFileMode.UserWrite);
            return Unit.Value;
        }
        var user = Environment.GetEnvironmentVariable("USERNAME");
        if (string.IsNullOrEmpty(user))
        {
            return Result.Fail("USERNAME is not set, so the token file cannot be limited to you");
        }
        var domain = Environment.GetEnvironmentVariable("USERDOMAIN");
        var account = string.IsNullOrEmpty(domain) ? user : $"{domain}\\{user}";
        var start = new ProcessStartInfo("icacls") { UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true };
        foreach (var arg in new[] { path, "/inheritance:r", "/grant:r", $"{account}:F" })
        {
            start.ArgumentList.Add(arg);
        }
        using var process = Process.Start(start);
        if (process is not null)
        {
            process.StandardOutput.ReadToEnd();
            process.StandardError.ReadToEnd();
            process.WaitForExit();
            if (process.ExitCode == 0)
            {
                return Unit.Value;
            }
        }
        // A token others may read is not kept.
        File.Delete(path);
        return Result.Fail($"icacls could not limit {path} to {account}");
    }
}

/// <summary>A client of a running editor's control channel.</summary>
public sealed class ControlClient : IDisposable
{
    private readonly TcpClient client;
    private readonly NetworkStream stream;
    private ulong nextId = 1;

    private ControlClient(TcpClient client)
    {
        this.client = client;
        stream = client.GetStream();
    }

    /// <summary>Connects to <paramref name="address"/> (loopback only) and authenticates with <paramref name="token"/>.</summary>
    public static Result<ControlClient, string> Connect(IPEndPoint address, string token)
    {
        if (!IPAddress.IsLoopback(address.Address))
        {
            return Result.Fail($"the control channel only listens on loopback addresses, not {address}");
        }
        var tcp = new TcpClient();
        try
        {
            if (!tcp.ConnectAsync(address.Address, address.Port).Wait(TimeSpan.FromSeconds(5)))
            {
                tcp.Dispose();
                return Result.Fail($"cannot reach the editor at {address}: no answer in 5 s");
            }
        }
        catch (AggregateException e) when (e.InnerException is SocketException inner)
        {
            tcp.Dispose();
            return Result.Fail($"cannot reach the editor at {address}: {inner.Message}");
        }
        tcp.ReceiveTimeout = (int)(ControlServer.ReplyTimeout + TimeSpan.FromSeconds(5)).TotalMilliseconds;
        var connected = new ControlClient(tcp);
        var auth = connected.Call("auth", new JsonObject { { "token", new JsonString(token) } });
        if (auth.IsError)
        {
            connected.Dispose();
            return auth.AsFailure();
        }
        return connected;
    }

    public void Dispose() => client.Dispose();

    /// <summary>Sends one request and waits for its reply.</summary>
    public Result<JsonValue, string> Call(string method, JsonValue parameters)
    {
        var id = nextId++;
        string? line;
        try
        {
            ControlServer.WriteLine(stream, new JsonObject { { "id", JsonNumber.Of(id) }, { "method", new JsonString(method) }, { "params", parameters } }.Canonical().ToCompact());
            line = ControlServer.ReadLine(stream);
        }
        catch (Exception e) when (e is IOException or SocketException or ObjectDisposedException)
        {
            return Result.Fail($"control channel: {e.Message}");
        }
        if (line is null)
        {
            return Result.Fail("the editor closed the control channel");
        }
        if (!JsonReader.Parse(line).TryGet(out var reply, out var error))
        {
            return Result.Fail($"control channel sent invalid JSON: {error}");
        }
        return reply.Get("ok")?.AsBool() == true ? reply.Get("result") ?? JsonValue.Null : Result.Fail(reply.Get("error")?.AsString() ?? "unknown error");
    }
}

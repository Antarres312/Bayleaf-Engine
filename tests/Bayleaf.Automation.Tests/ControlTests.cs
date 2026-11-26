using System.Net;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Automation.Tests;

public class ControlTests
{
    /// <summary>A server whose requests a background thread answers like the editor would.</summary>
    private static (ControlServer Server, CancellationTokenSource Stop) Server(string token)
    {
        var server = ControlServer.Start(0, token, () => { });
        var stop = new CancellationTokenSource();
        new Thread(() =>
        {
            while (!stop.IsCancellationRequested)
            {
                foreach (var request in server.Pending())
                {
                    Result<JsonValue, string> result = request.Method switch
                    {
                        "engine.commands" => new JsonArray([new JsonObject { { "id", new JsonString("app.version") } }]),
                        "engine.execute" => new JsonObject { { "ran", request.Params.Get("command") ?? JsonValue.Null } },
                        _ => Result.Fail($"unknown method {request.Method}"),
                    };
                    request.Respond(result);
                }
                Thread.Sleep(5);
            }
        })
        { IsBackground = true }.Start();
        return (server, stop);
    }

    [Fact]
    public void AWrongTokenIsRefusedAndTheRightOneWorks()
    {
        const string token = "secret-token-of-enough-length-0123456789";
        var (server, stop) = Server(token);
        using (server)
        using (stop)
        {
            Assert.Contains("authentication required", ControlClient.Connect(server.Address, "guess").Error, StringComparison.Ordinal);
            using var client = ControlClient.Connect(server.Address, token).Value;
            Assert.Equal("app.version", client.Call("engine.commands", JsonValue.Null).Value.AsArray()![0].Get("id")!.AsString());
            Assert.Equal("app.version", client.Call("engine.execute", new JsonObject { { "command", new JsonString("app.version") } }).Value.Get("ran")!.AsString());
            Assert.True(client.Call("auth", new JsonObject { { "token", new JsonString("x") } }).IsError, "no second auth");
            Assert.True(client.Call("format.disk", JsonValue.Null).IsError);
            stop.Cancel();
        }
    }

    [Fact]
    public void OnlyLoopbackAddressesAreAcceptedByTheClient() =>
        Assert.Contains("loopback", ControlClient.Connect(new IPEndPoint(IPAddress.Parse("192.0.2.1"), 7878), "x").Error, StringComparison.Ordinal);

    [Fact]
    public void TokensAreCreatedOnceAndThenReused()
    {
        var folder = Path.Join(Path.GetTempPath(), $"bayleaf-token-cs-{Environment.ProcessId}");
        var path = Path.Join(folder, "control.token");
        try
        {
            var first = ControlServer.LoadOrCreateToken(path).Value;
            Assert.Equal(64, first.Length);
            if (OperatingSystem.IsWindows())
            {
                // Only the current user is left on the file's access list.
                using var listing = System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo("icacls", [path]) { RedirectStandardOutput = true, UseShellExecute = false })!;
                var text = listing.StandardOutput.ReadToEnd();
                listing.WaitForExit();
                Assert.Equal(1, text.Split(":(").Length - 1);
            }
            Assert.Equal(first, ControlServer.LoadOrCreateToken(path).Value);
        }
        finally
        {
            Directory.Delete(folder, true);
        }
    }
}

using Bayleaf.Core.Json;
using Bayleaf.Engine;

namespace Bayleaf.Automation.Tests;

public sealed class McpTests : IDisposable
{
    private readonly string dir = Path.Join(Path.GetTempPath(), $"bayleaf-mcp-cs-{Environment.ProcessId}-{Guid.NewGuid():N}");

    public McpTests() => Directory.CreateDirectory(dir);

    public void Dispose() => Directory.Delete(dir, true);

    private LocalBackend Backend() => new(Confinement.Create([dir], [dir]).Value);

    private static JsonValue Json(string text) => JsonReader.Parse(text).Value;

    private static JsonValue Send(McpServer server, IBackend backend, string message) => Json(server.Handle(backend, message)!);

    private static JsonValue Call(IBackend backend, string message) => Send(new McpServer(), backend, message);

    private static string Text(JsonValue reply) => reply.Get("result")!.Get("content")!.AsArray()![0].Get("text")!.AsString()!;

    [Fact]
    public void InitializeAgreesOnAVersionAndOffersTools()
    {
        var b = Backend();
        var reply = Call(b, """{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}""");
        Assert.Equal("2025-06-18", reply.Get("result")!.Get("protocolVersion")!.AsString());
        Assert.IsType<JsonObject>(reply.Get("result")!.Get("capabilities")!.Get("tools"));
        var unknown = Call(b, """{"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}""");
        Assert.Equal(McpServer.ProtocolVersions[0], unknown.Get("result")!.Get("protocolVersion")!.AsString());
        Assert.Null(new McpServer().Handle(b, """{"jsonrpc":"2.0","method":"notifications/initialized"}"""));
    }

    [Fact]
    public void EveryCommandIsAToolAndCallsReachTheSession()
    {
        var b = Backend();
        var tools = Call(b, """{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}""").Get("result")!.Get("tools")!.AsArray()!;
        var create = tools.First(t => t.Get("name")!.AsString() == "scene_object_create");
        Assert.Equal("object", create.Get("inputSchema")!.Get("type")!.AsString());
        Assert.Equal(true, tools.First(t => t.Get("name")!.AsString() == "scene_objects").Get("annotations")!.Get("readOnlyHint")!.AsBool());
        Assert.All(tools, t => Assert.True(t.Get("name")!.AsString()!.All(c => char.IsAsciiLetterOrDigit(c) || c == '_'), "names suit every client"));

        var created = Call(b, """{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {"name": "From MCP"}}}""");
        Assert.Equal(false, created.Get("result")!.Get("isError")!.AsBool());
        var objects = Call(b, """{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scene_objects", "arguments": {}}}""");
        Assert.Contains("From MCP", Text(objects), StringComparison.Ordinal);
    }

    [Fact]
    public void FailuresAreReportedToTheModelAndBadMessagesToTheClient()
    {
        var b = Backend();
        var outside = Call(b, """{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "../../escape.bscene"}}}""");
        Assert.Equal(true, outside.Get("result")!.Get("isError")!.AsBool());
        Assert.Contains("outside the allowed folders", Text(outside), StringComparison.Ordinal);
        Assert.Equal(true, Call(b, """{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "format_disk"}}""").Get("result")!.Get("isError")!.AsBool());
        Assert.Equal(-32601.0, Call(b, """{"jsonrpc": "2.0", "id": 3, "method": "prompts/list"}""").Get("error")!.Get("code")!.AsDouble());
        Assert.Equal(-32700.0, Json(new McpServer().Handle(b, "{not json")!).Get("error")!.Get("code")!.AsDouble());
    }

    [Fact]
    public void ResourcesShowTheSceneAndSubscribersHearWhenItChanges()
    {
        var b = Backend();
        var server = new McpServer();
        var uris = Send(server, b, """{"jsonrpc": "2.0", "id": 1, "method": "resources/list"}""").Get("result")!.Get("resources")!.AsArray()!.Select(r => r.Get("uri")!.AsString()).ToList();
        Assert.Contains("bayleaf://scene", uris);
        Assert.Contains("bayleaf://scene/render", uris);

        Assert.IsType<JsonObject>(Send(server, b, """{"jsonrpc": "2.0", "id": 2, "method": "resources/subscribe", "params": {"uri": "bayleaf://scene"}}""").Get("result"));
        Assert.Empty(server.Changes(b));
        Send(server, b, """{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {"name": "Watched"}}}""");
        var notes = server.Changes(b);
        var note = Assert.Single(notes);
        Assert.Contains("notifications/resources/updated", note, StringComparison.Ordinal);
        Assert.Contains("bayleaf://scene", note, StringComparison.Ordinal);
        Assert.Empty(server.Changes(b));

        var read = Send(server, b, """{"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "bayleaf://scene"}}""");
        Assert.Contains("Watched", read.Get("result")!.Get("contents")!.AsArray()![0].Get("text")!.AsString(), StringComparison.Ordinal);
        Assert.Equal(-32002.0, Send(server, b, """{"jsonrpc": "2.0", "id": 5, "method": "resources/read", "params": {"uri": "file:///etc/passwd"}}""").Get("error")!.Get("code")!.AsDouble());
        Send(server, b, """{"jsonrpc": "2.0", "id": 6, "method": "resources/unsubscribe", "params": {"uri": "bayleaf://scene"}}""");
        Send(server, b, """{"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {}}}""");
        Assert.Empty(server.Changes(b));
    }

    /// <summary>Answers control requests from a session of its own, like the editor does.</summary>
    private sealed class SessionHandler(Session session) : IControlHandler
    {
        private readonly CommandRegistry registry = CommandRegistry.WithBuiltins();

        public Session Session { get; } = session;

        public JsonValue Commands() => registry.List(Session);

        public Core.Result<JsonValue, string> Execute(string command, JsonValue parameters) => registry.Execute(Session, command, parameters).MapError(e => e.Message);
    }

    [Fact]
    public async Task TheBridgeCarriesToolsResourcesAndRefusalsToTheRunningSession()
    {
        var handler = new SessionHandler(new Session { Confinement = Confinement.Create([dir], [dir]).Value });
        const string token = "bridge-test-token-0123456789abcdef0123456789";
        using var server = ControlServer.Start(0, token, () => { });
        var address = server.Address;

        // The MCP side, as `bayleaf-cli mcp --bridge` runs it.
        var agent = Task.Run(() =>
        {
            using var client = ControlClient.Connect(address, token).Value;
            var backend = new BridgeBackend(client);
            var mcp = new McpServer();
            JsonValue Go(string message) => Send(mcp, backend, message);
            return (Go("""{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}"""),
                Go("""{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {"name": "Bridged"}}}"""),
                Go("""{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "../../escape.bscene"}}}"""),
                Go("""{"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "bridged.bscene"}}}"""),
                Go("""{"jsonrpc": "2.0", "id": 5, "method": "resources/read", "params": {"uri": "bayleaf://scene"}}"""));
        });
        // The editor's side: answer between frames until the agent is done.
        while (!agent.IsCompleted)
        {
            server.Answer(handler);
            await Task.Delay(2, TestContext.Current.CancellationToken);
        }
        var (tools, created, refused, saved, read) = await agent;

        Assert.Contains(tools.Get("result")!.Get("tools")!.AsArray()!, t => t.Get("name")!.AsString() == "scene_object_create");
        Assert.Equal(false, created.Get("result")!.Get("isError")!.AsBool());
        Assert.Equal(true, refused.Get("result")!.Get("isError")!.AsBool());
        Assert.Equal(false, saved.Get("result")!.Get("isError")!.AsBool());
        Assert.True(File.Exists(Path.Join(dir, "bridged.bscene")));
        Assert.Contains("Bridged", read.Get("result")!.Get("contents")!.AsArray()![0].Get("text")!.AsString(), StringComparison.Ordinal);
        Assert.Contains(handler.Session.Scene.WorldMatrices(), m => handler.Session.Scene.NameOf(m.Id) == "Bridged");
    }
}

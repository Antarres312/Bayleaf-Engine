using System.Collections.Concurrent;

using Bayleaf.Core.Json;
using Bayleaf.Engine;

namespace Bayleaf.Automation;

/// <summary>Where tool calls go: a session of its own, or a running editor.</summary>
public interface IBackend
{
    /// <summary>The command list, as <c>engine.commands</c> returns it.</summary>
    Core.Result<List<JsonValue>, string> Commands();

    Core.Result<JsonValue, string> Execute(string command, JsonValue parameters);
}

/// <summary>A session of its own, confined to the given folders.</summary>
public sealed class LocalBackend(Confinement confinement) : IBackend
{
    private readonly CommandRegistry registry = CommandRegistry.WithBuiltins();

    public Session Session { get; } = new() { Confinement = confinement };

    public Core.Result<List<JsonValue>, string> Commands() => registry.List(Session).ToList();

    public Core.Result<JsonValue, string> Execute(string command, JsonValue parameters) => registry.Execute(Session, command, parameters).MapError(e => e.Message);
}

/// <summary>A running editor, through its control channel.</summary>
public sealed class BridgeBackend(ControlClient client) : IBackend
{
    public Core.Result<List<JsonValue>, string> Commands()
    {
        var listed = client.Call("engine.commands", JsonValue.Null);
        if (!listed.TryGet(out var value, out var error))
        {
            return Core.Result.Fail(error);
        }
        return value is JsonArray items ? items.ToList() : Core.Result.Fail($"engine.commands returned {value.ToCompact()}");
    }

    public Core.Result<JsonValue, string> Execute(string command, JsonValue parameters) =>
        client.Call("engine.execute", new JsonObject { { "command", new JsonString(command) }, { "params", parameters } });
}

/// <summary>
/// Model Context Protocol server: JSON-RPC 2.0 over stdio, one message per line. Every engine
/// command is a tool; dots in command ids become underscores in tool names
/// (<c>scene.object.create</c> → <c>scene_object_create</c>). One server is one client's connection:
/// it keeps the resources the client subscribed to, with a fingerprint of what each showed when it
/// was last reported.
/// </summary>
public sealed class McpServer
{
    /// <summary>Protocol versions this server speaks, newest first.</summary>
    public static readonly IReadOnlyList<string> ProtocolVersions = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

    /// <summary>How often the server looks for changes to subscribed resources while no message arrives.</summary>
    public static readonly TimeSpan PollInterval = TimeSpan.FromMilliseconds(500);

    /// <summary>Commands that only read: their tools carry <c>readOnlyHint</c>.</summary>
    private static readonly string[] ReadOnly = ["app.version", "scene.objects", "scene.document", "asset.info", "graph.get", "scene.components", "scene.object.inspect", "play.status"];

    /// <summary>What the server can show as resources: URI, name, title, MIME type and description.</summary>
    private static readonly (string Uri, string Name, string Title, string Mime, string Description)[] Resources =
    [
        ("bayleaf://scene", "scene", "Scene", "application/json", "The open scene: every object and component, as the scene file holds them"),
        ("bayleaf://scene/render", "scene-render", "Scene picture", "image/png", "The scene from its first camera (or framing every object), 640 x 360"),
        ("bayleaf://graph", "graph", "Open graph", "application/json", "The graph open in the graph editor, its file and its problems"),
        ("bayleaf://play", "play", "Play mode", "application/json", "Whether play mode runs, its time, and the scripts' errors"),
    ];

    private readonly SortedDictionary<string, ulong> subscriptions = new(StringComparer.Ordinal);

    /// <summary>Tool name for a command id.</summary>
    public static string ToolName(string command) => command.Replace('.', '_');

    /// <summary>The command a resource reads, its params, and the command whose result shows it changed.</summary>
    private static (string Command, JsonValue Params, string Watch)? ResourceCommand(string uri) => uri switch
    {
        "bayleaf://scene" => ("scene.document", JsonValue.Null, "scene.document"),
        "bayleaf://scene/render" => ("scene.render", new JsonObject { { "width", JsonNumber.Of(640L) }, { "height", JsonNumber.Of(360L) } }, "scene.document"),
        "bayleaf://graph" => ("graph.get", JsonValue.Null, "graph.get"),
        "bayleaf://play" => ("play.status", JsonValue.Null, "play.status"),
        _ => null,
    };

    /// <summary>Handles one JSON-RPC message; the reply line, or null for a notification.</summary>
    public string? Handle(IBackend backend, string line)
    {
        var parsed = JsonReader.Parse(line);
        if (!parsed.TryGet(out var message, out var parseError))
        {
            return ErrorReply(JsonValue.Null, -32700, $"parse error: {parseError}");
        }
        // A message without an id is a notification: never answered.
        if (message.Get("id") is not { } id)
        {
            return null;
        }
        var method = message.Get("method")?.AsString() ?? "";
        var parameters = message.Get("params") ?? JsonValue.Null;
        var uri = parameters.Get("uri")?.AsString() ?? "";
        if (method == "tools/call")
        {
            return Reply(id, ToolsCall(backend, parameters));
        }
        (JsonValue? Result, (long Code, string Message)? Error) outcome = method switch
        {
            "initialize" => (Initialize(parameters), null),
            "ping" => (new JsonObject(), null),
            "tools/list" => ToolsList(backend),
            "resources/list" => (ResourcesList(), null),
            "resources/templates/list" => (new JsonObject { { "resourceTemplates", new JsonArray() } }, null),
            "resources/read" => ResourcesRead(backend, uri),
            "resources/subscribe" => Subscribe(backend, uri),
            "resources/unsubscribe" => Unsubscribe(uri),
            "" => (null, (-32600, "invalid request: no method")),
            _ => (null, (-32601, $"method not found: {method}")),
        };
        return outcome.Error is var (code, text) ? ErrorReply(id, code, text) : Reply(id, outcome.Result!);
    }

    private (JsonValue?, (long, string)?) Subscribe(IBackend backend, string uri)
    {
        if (Fingerprint(backend, uri) is not { } print)
        {
            return (null, NotFound(uri));
        }
        subscriptions[uri] = print;
        return (new JsonObject(), null);
    }

    private (JsonValue?, (long, string)?) Unsubscribe(string uri)
    {
        subscriptions.Remove(uri);
        return (new JsonObject(), null);
    }

    /// <summary><c>notifications/resources/updated</c> for each subscribed resource that changed since it was last reported.</summary>
    public List<string> Changes(IBackend backend)
    {
        var notes = new List<string>();
        foreach (var uri in subscriptions.Keys.ToList())
        {
            if (Fingerprint(backend, uri) is { } print && print != subscriptions[uri])
            {
                subscriptions[uri] = print;
                notes.Add(new JsonObject
                {
                    { "jsonrpc", new JsonString("2.0") },
                    { "method", new JsonString("notifications/resources/updated") },
                    { "params", new JsonObject { { "uri", new JsonString(uri) } } },
                }.Canonical().ToCompact());
            }
        }
        return notes;
    }

    /// <summary>
    /// Reads messages from <paramref name="input"/> and writes replies to <paramref name="output"/>
    /// until the input ends. While no message arrives, subscribed resources are checked every
    /// <see cref="PollInterval"/>, so changes made in the editor are reported too.
    /// </summary>
    public static void Serve(IBackend backend, TextReader input, TextWriter output)
    {
        using var lines = new BlockingCollection<string>();
        var reader = new Thread(() =>
        {
            try
            {
                while (input.ReadLine() is { } line)
                {
                    lines.Add(line);
                }
            }
            catch (IOException)
            {
                // The input broke: the same as its end.
            }
            lines.CompleteAdding();
        })
        { IsBackground = true, Name = "bayleaf mcp input" };
        reader.Start();
        var server = new McpServer();
        while (!lines.IsCompleted)
        {
            var replies = new List<string>();
            if (lines.TryTake(out var line, PollInterval))
            {
                if (line.Trim().Length > 0 && server.Handle(backend, line) is { } reply)
                {
                    replies.Add(reply);
                }
            }
            replies.AddRange(server.Changes(backend));
            foreach (var reply in replies)
            {
                output.WriteLine(reply);
            }
            output.Flush();
        }
    }

    private static (long, string) NotFound(string uri) => (-32002, $"resource not found: {uri}");

    private static JsonObject ResourcesList() => new()
    {
        {
            "resources", new JsonArray(Resources.Select(r => (JsonValue)new JsonObject
            {
                { "uri", new JsonString(r.Uri) }, { "name", new JsonString(r.Name) }, { "title", new JsonString(r.Title) },
                { "mimeType", new JsonString(r.Mime) }, { "description", new JsonString(r.Description) },
            }))
        },
    };

    private static (JsonValue?, (long, string)?) ResourcesRead(IBackend backend, string uri)
    {
        if (ResourceCommand(uri) is not var (command, parameters, _))
        {
            return (null, NotFound(uri));
        }
        var result = backend.Execute(command, parameters);
        if (!result.TryGet(out var value, out var error))
        {
            return (null, (-32603, error));
        }
        var content = value.Get("png")?.AsString() is { } png
            ? new JsonObject { { "uri", new JsonString(uri) }, { "mimeType", new JsonString("image/png") }, { "blob", new JsonString(png) } }
            : new JsonObject { { "uri", new JsonString(uri) }, { "mimeType", new JsonString("application/json") }, { "text", new JsonString(value.ToPretty()) } };
        return (new JsonObject { { "contents", new JsonArray([content]) } }, null);
    }

    /// <summary>A hash of what shows whether <paramref name="uri"/> changed; null for an unknown resource. A command that fails (no graph open) hashes its error, so opening one counts as a change.</summary>
    private static ulong? Fingerprint(IBackend backend, string uri)
    {
        if (ResourceCommand(uri) is not var (_, _, watch))
        {
            return null;
        }
        var shown = backend.Execute(watch, JsonValue.Null);
        var text = shown.TryGet(out var value, out var error) ? value.ToCompact() : error;
        var hash = 0xcbf29ce484222325UL;
        foreach (var b in System.Text.Encoding.UTF8.GetBytes(text))
        {
            hash = (hash ^ b) * 0x100000001b3UL;
        }
        return hash;
    }

    private static JsonObject Initialize(JsonValue parameters)
    {
        var requested = parameters.Get("protocolVersion")?.AsString() ?? "";
        var version = ProtocolVersions.Contains(requested) ? requested : ProtocolVersions[0];
        return new JsonObject
        {
            { "protocolVersion", new JsonString(version) },
            {
                "capabilities", new JsonObject
                {
                    { "tools", new JsonObject { { "listChanged", JsonBool.False } } },
                    { "resources", new JsonObject { { "subscribe", JsonBool.True }, { "listChanged", JsonBool.False } } },
                }
            },
            { "serverInfo", new JsonObject { { "name", new JsonString("bayleaf") }, { "title", new JsonString("Bayleaf Engine") }, { "version", new JsonString(CommandRegistry.EngineVersion) } } },
            {
                "instructions", new JsonString("Every Bayleaf Engine command is a tool. Start with scene_objects to see the scene. "
                    + "Object ids are 32-character hex strings. File paths are relative to the first write root, and files "
                    + "outside the read and write roots are refused.")
            },
        };
    }

    private static (JsonValue?, (long, string)?) ToolsList(IBackend backend)
    {
        if (!backend.Commands().TryGet(out var commands, out var error))
        {
            return (null, (-32603, error));
        }
        var tools = new JsonArray();
        foreach (var c in commands)
        {
            if (c.Get("id")?.AsString() is not { } id)
            {
                continue;
            }
            var label = c.Get("label")?.AsString() ?? id;
            var parameters = c.Get("params")?.AsString() ?? "none";
            var tool = new JsonObject
            {
                { "name", new JsonString(ToolName(id)) },
                { "title", new JsonString(label) },
                { "description", new JsonString($"{label}. Params: {parameters}. (Bayleaf command `{id}`.)") },
                { "inputSchema", new JsonObject { { "type", new JsonString("object") }, { "additionalProperties", JsonBool.True } } },
            };
            if (ReadOnly.Contains(id))
            {
                tool.Add("annotations", new JsonObject { { "readOnlyHint", JsonBool.True } });
            }
            tools.Add(tool);
        }
        return (new JsonObject { { "tools", tools } }, null);
    }

    /// <summary>Tool results report failures in the result (<c>isError</c>), so the model sees them.</summary>
    private static JsonObject ToolsCall(IBackend backend, JsonValue parameters)
    {
        var name = parameters.Get("name")?.AsString() ?? "";
        var arguments = parameters.Get("arguments") ?? JsonValue.Null;
        if (!backend.Commands().TryGet(out var commands, out var error))
        {
            return ToolResult(error, true);
        }
        var command = commands.Select(c => c.Get("id")?.AsString()).FirstOrDefault(id => id is not null && ToolName(id) == name);
        if (command is null)
        {
            return ToolResult($"unknown tool: {name}", true);
        }
        var result = backend.Execute(command, arguments);
        return result.TryGet(out var value, out var failure) ? ToolResult(value.ToPretty(), false) : ToolResult(failure, true);
    }

    private static JsonObject ToolResult(string text, bool isError) => new()
    {
        { "content", new JsonArray([new JsonObject { { "type", new JsonString("text") }, { "text", new JsonString(text) } }]) },
        { "isError", JsonValue.From(isError) },
    };

    private static string Reply(JsonValue id, JsonValue result) =>
        new JsonObject { { "jsonrpc", new JsonString("2.0") }, { "id", id }, { "result", result } }.Canonical().ToCompact();

    private static string ErrorReply(JsonValue id, long code, string message) => new JsonObject
    {
        { "jsonrpc", new JsonString("2.0") },
        { "id", id },
        { "error", new JsonObject { { "code", JsonNumber.Of(code) }, { "message", new JsonString(message) } } },
    }.Canonical().ToCompact();
}

using Bayleaf.Core.Json;

namespace Bayleaf.Cli.Tests;

/// <summary>The <c>mcp</c> subcommand, driven like an MCP client would drive it over stdio.</summary>
public class McpCliTests
{
    [Fact]
    public void McpRefusesToStartWithoutBothRoots()
    {
        using var output = new StringWriter();
        using var errors = new StringWriter();
        Assert.NotEqual(0, Cli.Run(["mcp"], TextReader.Null, output, errors));
        Assert.Contains("automation-read-root", errors.ToString(), StringComparison.Ordinal);
    }

    [Fact]
    public void McpOverStdioCreatesSavesAndStaysInsideTheRoots()
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-cli-cs-mcp-{Environment.ProcessId}");
        Directory.CreateDirectory(dir);
        try
        {
            string[] messages =
            [
                """{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "it", "version": "1"}}}""",
                """{"jsonrpc": "2.0", "id": 2, "method": "tools/list"}""",
                """{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {"name": "Agent Cube", "components": {"mesh": null}}}}""",
                """{"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "agent.bscene"}}}""",
                """{"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "../escaped.bscene"}}}""",
            ];
            using var input = new StringReader(string.Join('\n', messages) + "\n");
            using var output = new StringWriter();
            using var errors = new StringWriter();
            Assert.Equal(0, Cli.Run(["mcp", "--automation-read-root", dir, "--automation-write-root", dir], input, output, errors));
            var replies = output.ToString().Split('\n', StringSplitOptions.RemoveEmptyEntries).Select(l => JsonReader.Parse(l).Value).ToList();
            Assert.Equal(5, replies.Count);
            Assert.Equal("bayleaf", replies[0].Get("result")!.Get("serverInfo")!.Get("name")!.AsString());
            Assert.True(replies[1].Get("result")!.Get("tools")!.AsArray()!.Count > 20);
            Assert.Equal(false, replies[2].Get("result")!.Get("isError")!.AsBool());
            Assert.Equal(false, replies[3].Get("result")!.Get("isError")!.AsBool());
            Assert.Equal(true, replies[4].Get("result")!.Get("isError")!.AsBool());
            Assert.Contains("Agent Cube", File.ReadAllText(Path.Join(dir, "agent.bscene")), StringComparison.Ordinal);
            Assert.False(File.Exists(Path.Join(Path.GetDirectoryName(dir)!, "escaped.bscene")));
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }
}

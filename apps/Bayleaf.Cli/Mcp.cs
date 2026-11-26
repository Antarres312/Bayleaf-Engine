using System.Net;

using Bayleaf.Automation;
using Bayleaf.Core.Json;
using Bayleaf.Engine;

namespace Bayleaf.Cli;

/// <summary>
/// <c>bayleaf-cli mcp</c>: an MCP server on stdio for AI agents. It refuses to start without both
/// automation roots, and refuses file access outside them (AGENTS.md rule 18). With <c>--bridge</c>
/// it drives a running editor through its control channel instead of a session of its own.
/// </summary>
internal static class Mcp
{
    private const string Usage = "Usage: bayleaf-cli mcp --automation-read-root <READ_ROOTS> --automation-write-root <WRITE_ROOTS> [--bridge <BRIDGE>] [--control-token-file <TOKEN_FILE>] [--scene <SCENE>]";

    public static int Run(List<string> args, TextReader input, TextWriter output, TextWriter errors)
    {
        var (read, write) = (new List<string>(), new List<string>());
        string? bridge = null, tokenFile = null, scene = null;
        for (var i = 0; i < args.Count; i++)
        {
            var name = args[i];
            if (name is "-h" or "--help")
            {
                output.WriteLine(Usage);
                return 0;
            }
            if (!name.StartsWith("--", StringComparison.Ordinal) || i + 1 >= args.Count)
            {
                errors.WriteLine(name.StartsWith("--", StringComparison.Ordinal)
                    ? $"error: a value is required for '{name}' but none was supplied\n\n{Usage}"
                    : $"error: unexpected argument '{name}' found\n\n{Usage}");
                return 2;
            }
            var value = args[++i];
            switch (name)
            {
                case "--automation-read-root":
                    read.Add(value);
                    break;
                case "--automation-write-root":
                    write.Add(value);
                    break;
                case "--bridge":
                    bridge = value;
                    break;
                case "--control-token-file":
                    tokenFile = value;
                    break;
                case "--scene":
                    scene = value;
                    break;
                default:
                    errors.WriteLine($"error: unexpected argument '{name}' found\n\n{Usage}");
                    return 2;
            }
        }
        var missing = new List<string>();
        if (read.Count == 0)
        {
            missing.Add("  --automation-read-root <READ_ROOTS>");
        }
        if (write.Count == 0)
        {
            missing.Add("  --automation-write-root <WRITE_ROOTS>");
        }
        if (missing.Count > 0)
        {
            errors.WriteLine($"error: the following required arguments were not provided:\n{string.Join('\n', missing)}\n\n{Usage}");
            return 2;
        }
        if (tokenFile is not null && bridge is null)
        {
            errors.WriteLine($"error: the following required arguments were not provided:\n  --bridge <BRIDGE>\n\n{Usage}");
            return 2;
        }
        if (scene is not null && bridge is not null)
        {
            errors.WriteLine($"error: the argument '--scene <SCENE>' cannot be used with '--bridge <BRIDGE>'\n\n{Usage}");
            return 2;
        }
        // Checked in both modes, so the roots are always real folders (AA 18).
        if (!Confinement.Create(read, write).TryGet(out var confinement, out var refused))
        {
            errors.WriteLine($"Error: {refused}");
            return 1;
        }
        if (bridge is not null)
        {
            if (!IPEndPoint.TryParse(bridge, out var address) || address.Port == 0)
            {
                errors.WriteLine($"error: invalid value '{bridge}' for '--bridge <BRIDGE>': invalid socket address syntax");
                return 2;
            }
            if (tokenFile is null)
            {
                errors.WriteLine("Error: --bridge needs --control-token-file, the token file the editor printed");
                return 1;
            }
            string token;
            try
            {
                token = File.ReadAllText(tokenFile).Trim();
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                errors.WriteLine($"Error: cannot read {tokenFile}\n\nCaused by:\n    {e.Message}");
                return 1;
            }
            if (!ControlClient.Connect(address, token).TryGet(out var client, out var unreachable))
            {
                errors.WriteLine($"Error: {unreachable}");
                return 1;
            }
            using (client)
            {
                errors.WriteLine($"bayleaf mcp: bridged to the editor at {address}");
                McpServer.Serve(new BridgeBackend(client), input, output);
            }
            return 0;
        }
        var backend = new LocalBackend(confinement);
        if (scene is not null && backend.Execute("scene.open", new JsonObject { { "path", new JsonString(scene) } }).TryGet(out _, out var notOpened) is false)
        {
            errors.WriteLine($"Error: {notOpened}");
            return 1;
        }
        errors.WriteLine("bayleaf mcp: ready on stdio");
        McpServer.Serve(backend, input, output);
        return 0;
    }
}

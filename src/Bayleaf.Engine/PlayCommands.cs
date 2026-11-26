using System.Globalization;
using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;
using Bayleaf.Script;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Engine;

/// <summary>
/// Play mode's commands (docs/automation.md): starting, stepping and stopping it, what scripts see as
/// held, messages to scripts, the game's state, scenes loaded while playing, the Input Maps in play and
/// world streaming. Play mode never records undo steps or marks the file changed, and stopping puts the
/// scene back as it was.
/// </summary>
internal static class PlayCommands
{
    /// <summary>Most steps one <c>play.step</c> may run.</summary>
    private const uint MaxSteps = 100_000;

    public static string? Playing(Session session) => session.IsPlaying ? null : "play mode is not running";

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "play.start", Label = "Play", Menu = "Play/Play", Shortcut = "Ctrl+P",
            Params = "{scripts?: \"interpreted\" | \"compiled\"} (default interpreted); returns {scripts, running, runtime}",
            Enabled = s => s.IsPlaying ? "play mode is already running" : null, Run = Start,
        },
        new() { Id = "play.stop", Label = "Stop", Menu = "Play/Stop", Shortcut = "Ctrl+P", Params = "none; restores the scene as it was before play", Enabled = Playing, Run = StopCommand },
        new() { Id = "play.pause", Label = "Pause or Resume", Menu = "Play/Pause or Resume", Shortcut = "Ctrl+Shift+P", Params = "none; toggles pause", Enabled = Playing, Run = Pause },
        new() { Id = "play.step", Label = "Step", Menu = "Play/Step", Params = "{seconds? = 1/60, steps? = 1}; advances play mode, also when paused", Enabled = Playing, Run = StepCommand },
        new()
        {
            Id = "play.input", Label = "Set Held Keys",
            Params = "{keys: [names], pointer?: [x, y] (-1 to 1, +y up; null when outside the view), buttons?: n (bit 0 left, 1 right, 2 middle), pad?: {axes: [left x, left y, right x, right y, left trigger, right trigger], buttons: [South, East, …]}}; what scripts see as held, until the next call or the next editor frame",
            Enabled = Playing, Run = Input,
        },
        new()
        {
            Id = "game.state", Label = "Game State",
            Params = "none; the game's values and each player's (both empty at first), saved values kept between plays (in the project's saves/progress.json), and outcome: \"won\", \"lost\" or null",
            Run = GameStateCommand,
        },
        new()
        {
            Id = "game.state.set", Label = "Set Game Value",
            Params = "{name, value, player?, saved?}: sets a value in the game's state, a player's (an object with Player Control) or, with saved: true, a saved one; play mode only",
            Enabled = Playing, Run = GameStateSet,
        },
        new()
        {
            Id = "play.scene.load", Label = "Load Scene in Play",
            Params = "{path, additive?}: a .bscene relative to the project; added to the playing scene, or in its place (scripts restart, the game state stays); returns {objects}",
            Enabled = Playing, Run = SceneLoad,
        },
        new() { Id = "play.scene.unload", Label = "Unload Scene in Play", Params = "{path}: removes the scenes added from that file while playing; returns {removed}", Enabled = Playing, Run = SceneUnload },
        new() { Id = "play.scene.list", Label = "Scenes Added in Play", Params = "none; returns {added: [{path, objects}]}", Enabled = Playing, Run = SceneList },
        new() { Id = "play.status", Label = "Play Status", Params = "none; returns {playing, paused, time, scripts, running, console}", Run = Status },
        new()
        {
            Id = "play.message", Label = "Send Message",
            Params = "{name, value?, id?}: sends the scripts of object id (or of every object) the message now, as a script's send does",
            Enabled = s => s.IsPlaying ? null : "messages go to scripts while the game plays", Run = Message,
        },
        new()
        {
            Id = "input.map.set", Label = "Set Input Map",
            Params = "{map, object?}: makes a map active, in the project's Input Map or an object's own (Input Map component); play mode only",
            Enabled = Playing, Run = InputMapSet,
        },
        new() { Id = "input.state", Label = "Input State", Params = "none; in play mode the active map of the project's Input Map and of each object's own, with their maps", Run = InputState },
        new()
        {
            Id = "stream.areas", Label = "Streamed Areas",
            Params = "none; returns {areas: [{id, name, scene, loadDistance, keepMargin, state, objects, problem?}]}: every Streamed Area in the playing scene; state is away, waiting, reading, here or failed",
            Enabled = Playing, Run = StreamAreas,
        },
        new()
        {
            Id = "stream.wait", Label = "Wait for Streamed Areas",
            Params = "{seconds?}: waits (at most seconds, default 10, up to 60) until no Streamed Area is being read, and adds those that arrived; returns {here, busy}",
            Enabled = Playing, Run = StreamWait,
        },
    ];

    private static JsonObject Start(Session session, JsonValue p)
    {
        var f = Params.Of(p, "scripts");
        // C# scripts always run compiled (Roslyn, then .NET's JIT): both choices run them so.
        f.Optional("scripts", v => JsonRead.Variant(v, ["interpreted", "compiled"]), 0);
        session.NoPrefabOpen("play mode");
        PlayLoop.ReadableModules(session, session.Scene);
        var snapshot = session.Scene.ToDoc();
        var (host, seen, problems) = PlayLoop.FreshHost(session, null);
        // Saved values from earlier plays, before any script starts.
        var saved = SavedValues.Read(session);
        if (saved.IsOk)
        {
            foreach (var (name, value) in saved.Value)
            {
                host.State.Saved[name] = value;
            }
        }
        var play = new PlayState(snapshot, host) { SavedOnDisk = saved.IsOk ? saved.Value : null, SeenScripts = seen };
        session.Play = play;
        System.Runtime.GCSettings.LatencyMode = System.Runtime.GCLatencyMode.SustainedLowLatency;
        var loaded = host.LoadScene(session.Scene, 0);
        var (scripts, running) = (host.Count, host.Running);
        var held = problems.Count > 0 && problems[0].StartsWith("Scripts did not run", StringComparison.Ordinal);
        foreach (var problem in TimelinePlay.StartUnder(session, null))
        {
            session.Log(Level.Error, problem);
        }
        session.Log(Level.Info, string.Create(CultureInfo.InvariantCulture, $"Play mode started: {running} of {scripts} scripts running."));
        foreach (var problem in problems)
        {
            session.Log(Level.Error, problem);
        }
        if (saved.IsError)
        {
            session.Log(Level.Error, $"Saved values could not be read, so play starts without them and they are not saved: {saved.Error}");
        }
        if (!loaded)
        {
            PlayLoop.StoppedByWatchdog(session);
        }
        else
        {
            AnimationCommands.StartPlay(session);
            PlayLoop.RunModules(session, (module, world) => module.Started(world));
            PlayLoop.Drain(session);
        }
        var result = J.Obj(("scripts", J.N(scripts)), ("running", J.N(running)), ("runtime", J.S("compiled")));
        if (held)
        {
            result.Add("notRun", J.S(problems[0]));
        }
        return result;
    }

    private static JsonObject StopCommand(Session session, JsonValue p)
    {
        Params.None(p);
        return J.Obj(("time", JsonNumber.Of(PlayLoop.StopPlay(session))));
    }

    private static PlayState Play(Session session) => session.Play ?? throw Stop.Failed("play mode is not running");

    private static JsonObject Pause(Session session, JsonValue p)
    {
        Params.None(p);
        var play = Play(session);
        play.Paused = !play.Paused;
        return J.Obj(("paused", J.B(play.Paused)));
    }

    private static JsonObject StepCommand(Session session, JsonValue p)
    {
        var f = Params.Of(p, "seconds", "steps");
        var seconds = f.Optional("seconds", JsonRead.F32, 1f / 60f);
        var steps = f.Optional("steps", JsonRead.U32, 1u);
        if (!(float.IsFinite(seconds) && seconds > 0f && seconds <= 1f))
        {
            throw Stop.BadParams("seconds must be more than 0 and at most 1");
        }
        if (steps is < 1 or > MaxSteps)
        {
            throw Stop.BadParams(string.Create(CultureInfo.InvariantCulture, $"steps must be between 1 and {MaxSteps}"));
        }
        for (var i = 0; i < steps && session.IsPlaying; i++)
        {
            PlayLoop.Step(session, seconds);
        }
        return J.Obj(("time", JsonNumber.Of(session.PlayTime)));
    }

    private static JsonObject Input(Session session, JsonValue p)
    {
        var f = Params.Of(p, "keys", "pointer", "buttons", "pad");
        var keys = f.Required("keys", v => JsonRead.List(v, JsonRead.Str));
        var pointer = f.Nullable("pointer", v => JsonRead.List(v, JsonRead.F32));
        var buttons = f.Optional("buttons", v => JsonRead.U32(v) is var n && n <= 255 ? (int)n : throw new JsonShapeException(string.Create(CultureInfo.InvariantCulture, $"invalid value: integer `{n}`, expected u8")), 0);
        var pad = f.NullableValue("pad", v =>
        {
            var g = JsonFields.Of(v, "struct PadParams", true, "axes", "buttons");
            return (Axes: g.Optional("axes", a => JsonRead.List(a, JsonRead.F32), [0f, 0f, 0f, 0f, 0f, 0f]), Buttons: g.Optional("buttons", b => JsonRead.List(b, JsonRead.Str), []));
        });
        if (pointer is { Count: not 2 })
        {
            throw new JsonShapeException($"invalid length {pointer.Count}, expected an array of length 2");
        }
        if (pad is { Axes.Count: not 6 })
        {
            throw new JsonShapeException($"invalid length {pad.Value.Axes.Count}, expected an array of length 6");
        }
        if (keys.Count > 64 || keys.Any(k => Text.Bytes(k) > 32))
        {
            throw Stop.BadParams("at most 64 keys of at most 32 characters");
        }
        if (pointer is { } at && !(float.IsFinite(at[0]) && float.IsFinite(at[1])))
        {
            throw Stop.BadParams("pointer must be two finite numbers");
        }
        Pad? held = null;
        if (pad is { } given)
        {
            var bits = 0u;
            foreach (var name in given.Buttons)
            {
                var index = PlayInput.PadButtons.ToList().IndexOf(name);
                if (index < 0)
                {
                    throw Stop.BadParams($"unknown gamepad button {Text.Debug(name)}; use {string.Join(", ", PlayInput.PadButtons)}");
                }
                bits |= 1u << index;
            }
            held = new Pad(given.Axes, bits);
        }
        Vector2? place = pointer is { } xy ? new Vector2(Math.Clamp(xy[0], -1f, 1f), Math.Clamp(xy[1], -1f, 1f)) : null;
        session.PlayInput = new PlayInput(new HashSet<string>(keys, StringComparer.Ordinal), place, buttons & 0b111, false, held);
        return [];
    }

    private static JsonObject Status(Session session, JsonValue p)
    {
        Params.None(p);
        var (scripts, running) = session.Play is { } play ? (play.Host.Count, play.Host.Running) : (0, 0);
        var tail = session.Console.Skip(Math.Max(0, session.Console.Count - 50)).Select(l => (JsonValue)J.Obj(("level", J.S(l.Level == Level.Error ? "error" : "info")), ("text", J.S(l.Text))));
        return J.Obj(
            ("playing", J.B(session.IsPlaying)),
            ("paused", J.B(session.IsPaused)),
            ("time", JsonNumber.Of(session.PlayTime)),
            ("scripts", J.N(scripts)),
            ("running", J.N(running)),
            ("particles", J.N(0)),
            ("console", new JsonArray(tail)));
    }

    private static JsonObject Message(Session session, JsonValue p)
    {
        var f = Params.Of(p, "name", "value", "id");
        var name = f.Required("name", JsonRead.Str);
        var value = f.Optional("value", JsonRead.F32, 0f);
        var id = Params.OptionalId(f, "id");
        if (name.Length == 0 || Text.Bytes(name) > ScriptHost.MaxMessageName || name.Any(char.IsControl))
        {
            throw Stop.BadParams(string.Create(CultureInfo.InvariantCulture, $"name needs 1 to {ScriptHost.MaxMessageName} bytes and no control characters"));
        }
        if (!float.IsFinite(value))
        {
            throw Stop.BadParams("value must be a finite number");
        }
        if (id is { } target && !session.Scene.Contains(target))
        {
            throw Stop.BadParams($"there is no object {target}");
        }
        var play = Play(session);
        play.Host.Send(session.Scene, id, name, value, play.Time);
        PlayLoop.AfterScripts(session);
        return J.Obj(("sent", J.S(name)));
    }

    /// <summary>The game's state as JSON. Outside play mode only saved values exist, as the project holds them.</summary>
    private static JsonObject StateJson(Session session)
    {
        GameState state;
        string? problem = null;
        if (session.Play is { } play)
        {
            state = play.Host.State;
        }
        else
        {
            state = new GameState();
            var saved = SavedValues.Read(session);
            if (saved.IsOk)
            {
                foreach (var (name, value) in saved.Value)
                {
                    state.Saved[name] = value;
                }
            }
            else
            {
                problem = saved.Error;
            }
        }
        static JsonObject Values(IEnumerable<KeyValuePair<string, double>> values)
        {
            var o = new JsonObject();
            foreach (var (name, value) in values)
            {
                o.Add(name, JsonNumber.Of(value));
            }
            return o;
        }
        var players = state.Players.Select(e => (JsonValue)J.Obj(("object", J.Id(e.Key)), ("name", J.S(session.Scene.NameOf(e.Key))), ("values", Values(e.Value))));
        return J.Obj(
            ("playing", J.B(session.IsPlaying)),
            ("game", Values(state.Game)),
            ("players", new JsonArray(players)),
            ("saved", Values(state.Saved)),
            ("savedFile", J.S(SavedValues.PathOf(session))),
            ("savedProblem", J.S(problem)),
            ("outcome", state.Outcome is { } won ? J.S(won ? "won" : "lost") : JsonValue.Null));
    }

    private static JsonObject GameStateCommand(Session session, JsonValue p)
    {
        Params.None(p);
        return StateJson(session);
    }

    private static JsonObject GameStateSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "name", "value", "player", "saved");
        var name = f.Required("name", JsonRead.Str);
        var value = f.Required("value", JsonRead.F64);
        var player = Params.OptionalId(f, "player");
        var saved = f.Optional("saved", JsonRead.Bool, false);
        var (scope, who) = (player, saved) switch
        {
            ({ }, true) => throw Stop.BadParams("give player or saved, not both"),
            ({ } id, false) when !session.Scene.TryGet<PlayerControl>(id, out _) => throw Stop.BadParams($"{id} is not a player (an object with Player Control)"),
            ({ } id, false) => (ValueScope.Player, id),
            (null, true) => (ValueScope.Saved, default(StableId)),
            _ => (ValueScope.Game, default(StableId)),
        };
        var play = session.Play ?? throw Stop.Failed("the game state exists only in play mode");
        if (!play.Host.State.Set(scope, who, name, value))
        {
            throw Stop.BadParams(string.Create(CultureInfo.InvariantCulture, $"give a name of 1 to 64 bytes and a finite number; a state holds at most {GameState.MaxValues} values"));
        }
        return StateJson(session);
    }

    private static JsonObject SceneLoad(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "additive");
        var path = f.Required("path", JsonRead.Str);
        var additive = f.Optional("additive", JsonRead.Bool, false);
        var objects = PlayLoop.LoadScene(session, path, additive).OrFail();
        return J.Obj(("objects", J.Ids(objects)));
    }

    private static JsonObject SceneUnload(Session session, JsonValue p)
    {
        var path = Params.Of(p, "path").Required("path", JsonRead.Str);
        return J.Obj(("removed", J.N(PlayLoop.UnloadScene(session, path).OrFail())));
    }

    private static JsonObject SceneList(Session session, JsonValue p)
    {
        Params.None(p);
        var added = session.Play?.Loaded.Select(l => (JsonValue)J.Obj(("path", J.S(l.Path)), ("objects", J.Ids(l.Roots)))) ?? [];
        return J.Obj(("added", new JsonArray(added)));
    }

    private static JsonObject InputMapSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "map", "object");
        var map = f.Required("map", JsonRead.Str);
        var obj = Params.OptionalId(f, "object");
        var play = session.Play ?? throw Stop.Failed("maps switch only in play mode");
        var state = obj is { } id ? play.Host.Input.Objects.GetValueOrDefault(id) : play.Host.Input.Project;
        if (state is null)
        {
            throw Stop.BadParams(obj is { } named ? $"{named} has no Input Map of its own" : "the project has no Input Map; choose one in Project Settings");
        }
        if (!state.SetActive(map))
        {
            throw Stop.BadParams($"there is no map {map}; the maps are {string.Join(", ", state.Maps.Maps)}");
        }
        return InputStateJson(session);
    }

    private static JsonObject InputState(Session session, JsonValue p)
    {
        Params.None(p);
        return InputStateJson(session);
    }

    private static JsonObject InputStateJson(Session session)
    {
        if (session.Play is not { } play)
        {
            return J.Obj(("playing", J.B(false)));
        }
        static JsonObject Describe(ActionMaps.State state) => J.Obj(("active", J.S(state.Active)), ("maps", J.Strings(state.Maps.Maps)));
        var objects = play.Host.Input.Objects.Select(e =>
        {
            var entry = Describe(e.Value);
            entry.Add("object", J.Id(e.Key));
            entry.Add("name", J.S(session.Scene.NameOf(e.Key)));
            return (JsonValue)entry;
        });
        return J.Obj(("playing", J.B(true)), ("project", play.Host.Input.Project is { } project ? Describe(project) : JsonValue.Null), ("objects", new JsonArray(objects)));
    }

    /// <summary>The world-streaming module, or the error a build without it gives.</summary>
    private static IStreamingModule Streaming(Session session) =>
        session.Modules.OfType<IStreamingModule>().FirstOrDefault() ?? throw Stop.Failed("this build has no world streaming");

    private static JsonObject StreamAreas(Session session, JsonValue p)
    {
        Params.None(p);
        var scene = session.Scene;
        var areas = ScriptHost.Order(scene).Where(id => scene.TryGet<StreamedArea>(id, out _)).ToList();
        var known = Streaming(session).Areas().ToDictionary(a => a.Area);
        var listed = areas.Select(id =>
        {
            scene.TryGet<StreamedArea>(id, out var area);
            var state = known.GetValueOrDefault(id);
            var entry = J.Obj(
                ("id", J.Id(id)),
                ("name", J.S(scene.NameOf(id) ?? "Object")),
                ("scene", J.S(area!.Scene)),
                ("loadDistance", JsonNumber.Of(area.LoadDistance)),
                ("keepMargin", JsonNumber.Of(area.KeepMargin)),
                ("state", J.S(state?.State ?? "away")),
                ("objects", J.Ids(state?.Objects ?? [])));
            if (state?.Problem is { } problem)
            {
                entry.Add("problem", J.S(problem));
            }
            return (JsonValue)entry;
        });
        return J.Obj(("areas", new JsonArray(listed)));
    }

    private static JsonObject StreamWait(Session session, JsonValue p)
    {
        var seconds = Params.Of(p, "seconds").Optional("seconds", JsonRead.F64, 10.0);
        if (!(double.IsFinite(seconds) && seconds >= 0 && seconds <= 60))
        {
            throw Stop.BadParams("seconds must be between 0 and 60");
        }
        var module = Streaming(session);
        var result = (Here: 0, Busy: 0);
        PlayLoop.RunModules(session, (m, world) =>
        {
            if (ReferenceEquals(m, module))
            {
                result = module.Wait(world, TimeSpan.FromSeconds(seconds));
            }
        });
        return J.Obj(("here", J.N(result.Here)), ("busy", J.N(result.Busy)));
    }
}

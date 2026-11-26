using System.Numerics;
using System.Reflection;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;
using Bayleaf.Script.Sdk;

using SceneModel = Bayleaf.Scene.Scene;
using Transform = Bayleaf.Scene.Components.Transform;

namespace Bayleaf.Script;

/// <summary>
/// The C# scripts of one play session (<c>docs/script-sdk.md</c>). Every script on every object gets its
/// own instance of its class; the scene's transforms live in packed arrays by the objects' places while
/// the scripts run, and reach the scene once per step for what moved. What scripts ask for that changes
/// the scene (spawns, destroys, animation, interfaces, scenes) comes back from <see cref="TakeRequests"/>
/// for the engine to do after the step, as the Rust host's requests do.
/// </summary>
/// <remarks>
/// Script code runs only on the host's script thread, inside <see cref="ScriptThread.Run"/>: the
/// engine waits for each step at most <see cref="TimeLimit"/>, and a step that takes longer leaves
/// <see cref="Stalled"/> set for the engine to stop play mode.
/// </remarks>
public sealed partial class ScriptHost : IDisposable
{
    /// <summary>Most requests and messages one script call may make.</summary>
    public const int MaxAsksPerCall = 64;

    /// <summary>Longest message name, in bytes.</summary>
    public const int MaxMessageName = 64;

    /// <summary>Longest text a single log call keeps, and most log lines kept between drains.</summary>
    private const int MaxLogBytes = 4096;

    private const int MaxLogLines = 1000;

    /// <summary>How long one step's scripts may take before the watchdog stops play mode.</summary>
    public static readonly TimeSpan DefaultTimeLimit = TimeSpan.FromSeconds(2);

    private readonly ScriptThread thread;
    private readonly World world;
    private readonly List<Instance> instances = [];
    private readonly List<Request> requests = [];
    private readonly List<ScriptFault> faults = [];
    private readonly List<LogLine> logs = [];
    private readonly View view = new();
    private List<Pending> messages = [];
    private readonly List<(StableId Object, string Key, string Name, Vector3 Value)> propertyWrites = [];
    private readonly HashSet<string> told = new(StringComparer.Ordinal);
    private LoadedScripts? scripts;
    private List<int> own = [];
    private List<ManyGroup> groups = [];
    private (int Version, ulong Structure) grouped = (-1, ulong.MaxValue);
    private int version;
    private List<Pending> later = [];

    // The work the script thread does next, set before each run so no closure is made per step.
    private readonly Action stepJob;
    private SceneModel? jobScene;
    private float jobSeconds;

    /// <param name="scripts">The project's compiled scripts; null when none run (none compiled, or the project's scripts are not allowed).</param>
    public ScriptHost(LoadedScripts? scripts)
    {
        this.scripts = scripts;
        world = new World(this);
        thread = new ScriptThread(world);
        stepJob = StepJob;
    }

    /// <summary>The longest one step's scripts may take.</summary>
    public TimeSpan TimeLimit { get; set; } = DefaultTimeLimit;

    /// <summary>The watchdog stopped a step that ran too long: play mode must stop. <see cref="StalledIn"/> says which script ran.</summary>
    public bool Stalled => thread.Stalled;

    /// <summary>The script that was running when the watchdog stopped the step.</summary>
    public string? StalledIn { get; private set; }

    /// <summary>Seconds since play mode started, as scripts read it.</summary>
    public double Time { get; private set; }

    /// <summary>The game's state every script shares.</summary>
    public GameState State { get; set; } = new();

    /// <summary>The Input Maps scripts read with <see cref="Controls.Action"/>.</summary>
    public InputSet Input { get; } = new();

    /// <summary>What the player holds now.</summary>
    public PlayInput Held { get; set; } = PlayInput.None;

    /// <summary>The project's data files as play read them when it started, by path relative to the project folder.</summary>
    public IReadOnlyDictionary<string, SortedDictionary<string, ResolvedField>> Data { get; set; } = new Dictionary<string, SortedDictionary<string, ResolvedField>>();

    /// <summary>Each object's named interface elements as they show: value and text.</summary>
    public IReadOnlyDictionary<(StableId Object, string Element), (double Value, string Text)> InterfaceValues { get; set; } = new Dictionary<(StableId, string), (double, string)>();

    /// <summary>The animated objects as the last step posed them.</summary>
    public IReadOnlyDictionary<StableId, AnimatedView> Animation { get; set; } = new Dictionary<StableId, AnimatedView>();

    /// <summary>Script instances loaded, running or stopped.</summary>
    public int Count => instances.Count;

    public int Running => instances.Count(i => !i.Stopped);

    /// <summary>The compiled scripts this host runs, if any.</summary>
    public LoadedScripts? Scripts => scripts;

    private sealed class Instance(StableId obj, int place, ScriptRef script, ScriptType type)
    {
        public StableId Object { get; } = obj;

        /// <summary>Its place among its object's scripts.</summary>
        public int Place { get; } = place;

        public string Name { get; } = script.DisplayName();

        public string Module { get; } = ScriptProject.Key(script.Module);

        /// <summary>Its search keys: the shown name and the file's stem.</summary>
        public string[] Keys { get; } = [Visual.KindKey(script.DisplayName()), Visual.KindKey(Path.GetFileNameWithoutExtension(script.Module))];

        public ScriptType Type { get; } = type;

        public GameScript? Script { get; set; }

        public bool Stopped { get; set; }

        public bool NeedPrepare { get; set; } = true;

        public bool NeedBegin { get; set; } = true;

        public int View { get; set; } = -1;

        /// <summary>The scene's values as last given to its fields, to see when the Inspector changes one.</summary>
        public SortedDictionary<string, PropertyValue> Applied { get; set; } = new(Utf8Order.Instance);

        /// <summary>A rule the script broke inside a call: it stops when the call ends, even when its code caught the error.</summary>
        public string? Broke { get; set; }
    }

    private sealed record Pending(StableId? To, string Name, float Value, EventValue[] Values);

    // ---- loading ---------------------------------------------------------------------------------

    /// <summary>
    /// Makes every script of every object of <paramref name="scene"/>, in hierarchy order, then runs
    /// their properties, <see cref="GameScript.Prepare"/> and <see cref="GameScript.Begin"/>, each phase for
    /// all before the next. Problems are faults; the other scripts still load. False when the watchdog stopped it.
    /// </summary>
    public bool LoadScene(SceneModel scene, double time) => LoadUnder(scene, null, time);

    /// <summary>As <see cref="LoadScene"/>, for <paramref name="roots"/> and everything under them (a placed prefab, an added scene); all of the scene when null.</summary>
    public bool LoadUnder(SceneModel scene, IReadOnlyCollection<StableId>? roots, double time)
    {
        Time = time;
        var first = instances.Count;
        foreach (var id in Order(scene))
        {
            if (roots is not null && !roots.Any(r => r == id || scene.IsInside(id, r)))
            {
                continue;
            }
            if (!scene.TryGet<Scripts>(id, out var list))
            {
                continue;
            }
            for (var place = 0; place < list.Items.Count; place++)
            {
                var script = list.Items[place];
                var (type, problem) = Resolve(script);
                if (type is null)
                {
                    faults.Add(new ScriptFault(id, script.DisplayName(), problem ?? ""));
                    continue;
                }
                instances.Add(new Instance(id, place, script, type));
            }
        }
        version++;
        if (instances.Count == first)
        {
            return true;
        }
        jobScene = scene;
        return Guard(() => StartFrom(scene, first));
    }

    /// <summary>The script type a scene's script runs, or why it cannot run.</summary>
    private (ScriptType? Type, string? Error) Resolve(ScriptRef script)
    {
        if (!ScriptProject.IsScript(script.Module))
        {
            return (null, $"{script.Module} is not a C# script (.cs): the C# engine runs C# scripts; visual scripts compile to C# in a later step, and .wasm modules are the Rust engine's");
        }
        if (scripts is null)
        {
            return (null, "the project's scripts do not run (see the console)");
        }
        var key = ScriptProject.Key(script.Module);
        if (scripts.Types.TryGetValue(key, out var type))
        {
            return (type, null);
        }
        return (null, scripts.Project.Describe(script.Module).TryGet(out var described, out var why)
            ? $"{script.Module}: {string.Join("; ", described.Problems)}"
            : why);
    }

    /// <summary>Constructs, sets the properties of, prepares and begins the instances from <paramref name="first"/> on. On the script thread.</summary>
    private void StartFrom(SceneModel scene, int first)
    {
        RefreshView(scene, first);
        for (var slot = first; slot < instances.Count; slot++)
        {
            var instance = instances[slot];
            Call(slot, "the constructor", () =>
            {
                var made = instance.Type.Create();
                made.World = world;
                made.Slot = slot;
                instance.Script = made;
            });
            if (!instance.Stopped)
            {
                ApplyProperties(scene, slot, Values(scene, instance));
            }
        }
        for (var slot = first; slot < instances.Count; slot++)
        {
            var instance = instances[slot];
            if (instance.NeedPrepare && !instance.Stopped)
            {
                instance.NeedPrepare = false;
                Call(slot, "Prepare", () => instance.Script!.Prepare());
            }
        }
        for (var slot = first; slot < instances.Count; slot++)
        {
            var instance = instances[slot];
            if (instance.NeedBegin && !instance.Stopped)
            {
                instance.NeedBegin = false;
                Call(slot, "Begin", () => instance.Script!.Begin());
            }
        }
        WritePoses(scene);
        ApplyPropertyWrites();
    }

    /// <summary>
    /// Runs every script anew with <paramref name="next"/>, the project's scripts compiled again: each
    /// instance is made again from its class, its properties set, and it prepares and begins. The scene
    /// keeps what play changed so far. False when the watchdog stopped it.
    /// </summary>
    public bool Reload(LoadedScripts? next, SceneModel scene, double time)
    {
        var old = scripts;
        scripts = next;
        instances.Clear();
        messages.Clear();
        propertyWrites.Clear();
        version++;
        var loaded = LoadScene(scene, time);
        if (old is not null && !ReferenceEquals(old, next))
        {
            old.Unload();
        }
        return loaded;
    }

    // ---- stepping --------------------------------------------------------------------------------

    /// <summary>
    /// One step: every running script's <see cref="GameScript.Advance"/> (or one
    /// <see cref="IAdvanceMany{TSelf}.AdvanceMany"/> per script type), the moved poses written to the
    /// scene, then the messages sent, in order. False when the watchdog stopped it.
    /// </summary>
    public bool Update(SceneModel scene, float seconds, double time)
    {
        Time = time;
        jobScene = scene;
        jobSeconds = seconds;
        return Run(stepJob);
    }

    private void StepJob()
    {
        var scene = jobScene!;
        var seconds = jobSeconds;
        RefreshView(scene, 0);
        if (grouped != (version, view.Structure))
        {
            Regroup(scene);
        }
        foreach (var slot in own)
        {
            var instance = instances[slot];
            if (instance.Stopped)
            {
                continue;
            }
            Begin(slot, 1);
            try
            {
                instance.Script!.Advance(seconds);
            }
            catch (Exception e) when (e is not OutOfMemoryException)
            {
                Stop(slot, "Advance", e);
            }
            End(slot);
        }
        foreach (var group in groups)
        {
            RunMany(group, seconds);
        }
        WritePoses(scene);
        ApplyPropertyWrites();
        Deliver(scene);
        ApplyPropertyWrites();
    }

    private void RunMany(ManyGroup group, float seconds)
    {
        var running = 0;
        for (var i = 0; i < group.Count; i++)
        {
            if (!instances[group.Slots[i]].Stopped)
            {
                running++;
            }
        }
        if (running == 0)
        {
            return;
        }
        var poses = view.Poses;
        for (var i = 0; i < group.Count; i++)
        {
            group.Poses[i] = poses[group.Views[i]];
        }
        var first = group.Slots[0];
        Begin(first, group.Count);
        try
        {
            group.Type.Many!(group, seconds);
        }
        catch (Exception e) when (e is not OutOfMemoryException)
        {
            // An error stops the script on every object it steps, with one fault on the first.
            for (var i = 1; i < group.Count; i++)
            {
                instances[group.Slots[i]].Stopped = true;
            }
            Stop(first, "AdvanceMany", e, group.Count - 1);
            return;
        }
        if (!End(first))
        {
            for (var i = 0; i < group.Count; i++)
            {
                instances[group.Slots[i]].Stopped = true;
            }
            return;
        }
        for (var i = 0; i < group.Count; i++)
        {
            ref var pose = ref group.Poses[i];
            var at = group.Views[i];
            if (pose != poses[at] && !instances[group.Slots[i]].Stopped)
            {
                if (!pose.IsFinite)
                {
                    for (var j = 0; j < group.Count; j++)
                    {
                        instances[group.Slots[j]].Stopped = true;
                    }
                    Fault(first, "AdvanceMany gave back a pose that is not finite; the script stopped on every object it steps");
                    return;
                }
                view.Move(at, pose);
            }
        }
    }

    /// <summary>Sorts the running instances for the step: those that step per object, and a group per script type that steps all of its objects in one call.</summary>
    private void Regroup(SceneModel scene)
    {
        own = [];
        var byType = new Dictionary<ScriptType, (List<int> Slots, List<int> Views)>();
        var order = new List<ScriptType>();
        for (var slot = 0; slot < instances.Count; slot++)
        {
            var instance = instances[slot];
            if (instance.Stopped)
            {
                continue;
            }
            var at = instance.View;
            if (instance.Type.Many is not null)
            {
                if (!byType.TryGetValue(instance.Type, out var lists))
                {
                    lists = ([], []);
                    byType[instance.Type] = lists;
                    order.Add(instance.Type);
                }
                lists.Slots.Add(slot);
                lists.Views.Add(at);
            }
            else if (instance.Type.Advances)
            {
                own.Add(slot);
            }
        }
        groups = [.. order.Select(type =>
        {
            var (slots, views) = byType[type];
            var array = Array.CreateInstance(type.Type, slots.Count);
            for (var i = 0; i < slots.Count; i++)
            {
                array.SetValue(instances[slots[i]].Script, i);
            }
            return new ManyGroup(type, array, [.. slots], [.. views]);
        })];
        CheckInspector(scene);
        grouped = (version, view.Structure);
    }

    /// <summary>The scene's values of the instances' properties changed (the Inspector, during play): give them to the fields again.</summary>
    private void CheckInspector(SceneModel scene)
    {
        for (var slot = 0; slot < instances.Count; slot++)
        {
            var instance = instances[slot];
            if (instance.Stopped || instance.Script is null)
            {
                continue;
            }
            var values = Values(scene, instance);
            if (values.Count != instance.Applied.Count || values.Any(v => !instance.Applied.TryGetValue(v.Key, out var was) || !was.Equals(v.Value)))
            {
                ApplyProperties(scene, slot, values);
            }
        }
    }

    /// <summary>The scene's properties of an instance's script, as the scene holds them now.</summary>
    private static SortedDictionary<string, PropertyValue> Values(SceneModel scene, Instance instance) =>
        scene.TryGet<Scripts>(instance.Object, out var list) && instance.Place < list.Items.Count && ScriptProject.Key(list.Items[instance.Place].Module) == instance.Module
            ? list.Items[instance.Place].Properties
            : instance.Applied;

    /// <summary>Gives the instance's fields the scene's values (or the declared defaults) and tells the script.</summary>
    private void ApplyProperties(SceneModel scene, int slot, SortedDictionary<string, PropertyValue> values)
    {
        var instance = instances[slot];
        instance.Applied = new SortedDictionary<string, PropertyValue>(values, Utf8Order.Instance);
        Call(slot, "setting its properties", () =>
        {
            foreach (var field in instance.Type.Description.Fields)
            {
                var declared = field.Declared;
                var value = values.TryGetValue(declared.Name, out var given) && declared.Accepts(given) ? given : declared.Default;
                if (instance.Type.Fields.TryGetValue(declared.Name, out var info))
                {
                    info.SetValue(instance.Script, FieldValue(info.FieldType, declared.Type, value));
                }
            }
        });
        if (!instance.Stopped)
        {
            Call(slot, "PropertiesChanged", () => instance.Script!.PropertiesChanged());
        }
    }

    /// <summary>A property's value as its field holds it.</summary>
    private object? FieldValue(Type field, PropertyType type, PropertyValue value)
    {
        if (type == PropertyType.SceneObject)
        {
            var name = value.AsText() ?? "";
            return name.Length > 0 && view.ByName.TryGetValue(name, out var at) ? new SceneObject(view.Ids[at].AsUInt128()) : SceneObject.None;
        }
        return value switch
        {
            PropertyValue.Number n when field == typeof(float) => (float)n.Value,
            PropertyValue.Number n when field == typeof(int) => (int)Math.Clamp(Math.Round(n.Value), int.MinValue, int.MaxValue),
            PropertyValue.Number n => n.Value,
            PropertyValue.Flag f => f.Value,
            PropertyValue.Triple t => t.Value,
            PropertyValue.Text t => t.Value,
            _ => null,
        };
    }

    /// <summary>Sets the script properties scripts asked for: on every script of the object with that name that declares the property.</summary>
    private void ApplyPropertyWrites()
    {
        while (propertyWrites.Count > 0)
        {
            var writes = propertyWrites.ToList();
            propertyWrites.Clear();
            foreach (var (obj, key, name, numbers) in writes)
            {
                for (var slot = 0; slot < instances.Count; slot++)
                {
                    var instance = instances[slot];
                    if (instance.Stopped || instance.Object != obj || !instance.Keys.Contains(key) || instance.Type.Description.Fields.FirstOrDefault(f => f.Declared.Name == name) is not { } field)
                    {
                        continue;
                    }
                    if (PropertyValue.FromNumbers(field.Declared.Type, numbers) is { } value && instance.Type.Fields.TryGetValue(name, out var info))
                    {
                        Call(slot, "setting its properties", () => info.SetValue(instance.Script, FieldValue(info.FieldType, field.Declared.Type, value)));
                        if (!instance.Stopped)
                        {
                            Call(slot, "PropertiesChanged", () => instance.Script!.PropertiesChanged());
                        }
                    }
                }
            }
        }
    }

    /// <summary>Delivers the messages sent so far, in the order sent; those sent while they are handled wait for the next step.</summary>
    private void Deliver(SceneModel scene)
    {
        if (messages.Count == 0)
        {
            return;
        }
        var now = messages;
        messages = later;
        messages.Clear();
        later = now;
        foreach (var message in now)
        {
            for (var slot = 0; slot < instances.Count; slot++)
            {
                var instance = instances[slot];
                if (instance.Stopped || !instance.Type.Receives || message.To is { } to && instance.Object != to || !scene.Contains(instance.Object))
                {
                    continue;
                }
                Begin(slot, 1);
                try
                {
                    instance.Script!.Receive(new Message(message.Name, message.Value, message.Values));
                }
                catch (Exception e) when (e is not OutOfMemoryException)
                {
                    Stop(slot, "Receive", e);
                }
                End(slot);
            }
        }
        now.Clear();
        WritePoses(scene);
    }

    /// <summary>Sends the scripts of <paramref name="to"/> (or of every object) a message now, as an Inspector button or <c>play.message</c> does. False when the watchdog stopped it.</summary>
    public bool Send(SceneModel scene, StableId? to, string name, float value, double time) => SendAll(scene, [(to, name, value, [])], time);

    /// <summary>Triggers each event on its object now, bringing the object as its one value: what a clip's marked moments become.</summary>
    public bool TriggerEvents(SceneModel scene, IEnumerable<(StableId Object, string Name)> events, double time) =>
        SendAll(scene, [.. events.Select(e => ((StableId?)e.Object, e.Name, 0f, new[] { EventValue.Of(new SceneObject(e.Object.AsUInt128())) }))], time);

    private bool SendAll(SceneModel scene, List<(StableId? To, string Name, float Value, EventValue[] Values)> sent, double time)
    {
        if (sent.Count == 0)
        {
            return true;
        }
        Time = time;
        foreach (var (to, name, value, values) in sent)
        {
            messages.Add(new Pending(to, name, value, values));
        }
        return Guard(() =>
        {
            RefreshView(scene, 0);
            Deliver(scene);
            ApplyPropertyWrites();
        });
    }

    // ---- calls and faults ------------------------------------------------------------------------

    private int current = -1;
    private int callAsks;
    private int callSends;
    private int callLimit = MaxAsksPerCall;

    /// <summary>Runs a job on the script thread under the watchdog. False when it took too long.</summary>
    private bool Run(Action job)
    {
        if (thread.Stalled)
        {
            return false;
        }
        if (thread.Run(job, TimeLimit))
        {
            return true;
        }
        StalledIn = current >= 0 && current < instances.Count ? $"{instances[current].Name} on {NameOfObject(instances[current].Object)}" : "a script";
        return false;
    }

    private bool Guard(Action job) => Run(job);

    /// <summary>Starts a call into the script in <paramref name="slot"/>: what it asks is counted against this call, for <paramref name="objects"/> objects.</summary>
    private void Begin(int slot, int objects)
    {
        current = slot;
        callAsks = 0;
        callSends = 0;
        callLimit = MaxAsksPerCall * objects;
    }

    /// <summary>Ends a call: a rule the script broke stops it. False when it stopped.</summary>
    private bool End(int slot)
    {
        current = -1;
        var instance = instances[slot];
        if (instance.Broke is { } broke)
        {
            instance.Broke = null;
            Fault(slot, broke);
            return false;
        }
        return !instance.Stopped;
    }

    private void Call(int slot, string what, Action call)
    {
        Begin(slot, 1);
        try
        {
            call();
        }
        catch (Exception e) when (e is not OutOfMemoryException)
        {
            Stop(slot, what, e);
        }
        End(slot);
    }

    private void Stop(int slot, string what, Exception error, int others = 0)
    {
        var inner = error is TargetInvocationException { InnerException: { } cause } ? cause : error;
        var instance = instances[slot];
        var reason = instance.Broke ?? $"{what}: {inner.GetType().Name}: {inner.Message}";
        instance.Broke = null;
        Fault(slot, others > 0 ? $"{reason} (it stopped on its {others} other objects too)" : reason);
    }

    private void Fault(int slot, string error)
    {
        var instance = instances[slot];
        instance.Stopped = true;
        version++;
        faults.Add(new ScriptFault(instance.Object, instance.Name, error));
    }

    /// <summary>A rule the calling script broke: it stops when the call ends, and the call it made fails now.</summary>
    private InvalidOperationException Broke(int slot, string why)
    {
        if (slot >= 0 && slot < instances.Count)
        {
            instances[slot].Broke ??= why;
        }
        return new InvalidOperationException(why);
    }

    /// <summary>Asks made by the call running now, against its limit; false when it already asked too much.</summary>
    private bool Asked()
    {
        if (callAsks >= callLimit)
        {
            return false;
        }
        callAsks++;
        return true;
    }

    private void AddLog(int slot, string text, bool error = false)
    {
        if (slot < 0 || slot >= instances.Count)
        {
            return;
        }
        var instance = instances[slot];
        logs.Add(new LogLine(instance.Object, instance.Name, Text.Bytes(text) <= MaxLogBytes ? text : text[..Math.Min(text.Length, MaxLogBytes)], error));
        if (logs.Count > MaxLogLines)
        {
            logs.RemoveRange(0, logs.Count - MaxLogLines);
        }
    }

    /// <summary>Says once per play that something scripts asked for is not in the C# engine yet.</summary>
    private void NotYet(int slot, string what)
    {
        if (told.Add(what))
        {
            AddLog(slot, $"{what} is not in the C# engine yet: it comes with stage 5 of docs/csharp-redesign.md, so the call did nothing", true);
        }
    }

    private string NameOfObject(StableId id) => view.Index.TryGetValue(id, out var at) ? view.Names[at] : "Object";

    public List<Request> TakeRequests()
    {
        var taken = requests.ToList();
        requests.Clear();
        return taken;
    }

    public List<ScriptFault> TakeFaults()
    {
        var taken = faults.ToList();
        faults.Clear();
        return taken;
    }

    public List<LogLine> TakeLogs()
    {
        var taken = logs.ToList();
        logs.Clear();
        return taken;
    }

    public void Dispose()
    {
        thread.Dispose();
        if (!thread.Stalled)
        {
            scripts?.Unload();
        }
    }

    // ---- the objects as scripts see them ---------------------------------------------------------

    /// <summary>The scene's objects in hierarchy order, parents before children.</summary>
    public static List<StableId> Order(SceneModel scene)
    {
        var output = new List<StableId>(scene.ObjectCount);
        var stack = new Stack<StableId>();
        var roots = scene.Children(null);
        for (var i = roots.Count - 1; i >= 0; i--)
        {
            stack.Push(roots[i]);
        }
        while (stack.TryPop(out var id))
        {
            output.Add(id);
            var children = scene.Children(id);
            for (var i = children.Count - 1; i >= 0; i--)
            {
                stack.Push(children[i]);
            }
        }
        return output;
    }

    private static Pose PoseOf(in Transform t) => new(new Vector3d(t.Position.X, t.Position.Y, t.Position.Z), t.Rotation, t.Scale);

    private static Transform TransformOf(Pose p) => new(new Double3(p.Position.X, p.Position.Y, p.Position.Z), p.Rotation, p.Scale);

    private void WritePoses(SceneModel scene) => view.Write(scene);

    /// <summary>
    /// Brings the view up to date. When the scene's structure changed, every instance finds its object's
    /// new place and those whose object went stop; else only the instances from <paramref name="first"/>
    /// on that have no place yet look.
    /// </summary>
    private void RefreshView(SceneModel scene, int first)
    {
        if (!view.Refresh(scene))
        {
            for (var slot = first; slot < instances.Count; slot++)
            {
                if (instances[slot].View < 0)
                {
                    Place(slot);
                }
            }
            return;
        }
        for (var slot = 0; slot < instances.Count; slot++)
        {
            Place(slot);
        }
    }

    private void Place(int slot)
    {
        var instance = instances[slot];
        if (view.Index.TryGetValue(instance.Object, out var at))
        {
            instance.View = at;
            return;
        }
        instance.View = -1;
        if (!instance.Stopped)
        {
            // Its object went: its scripts stop.
            instance.Stopped = true;
            version++;
        }
    }

    /// <summary>
    /// What scripts see of the scene: each object's id, name and transform by its place in hierarchy
    /// order. Kept from step to step while the scene's structure stays, with only what moved read again.
    /// </summary>
    private sealed class View
    {
        public ulong Structure { get; private set; } = ulong.MaxValue;

        public StableId[] Ids { get; private set; } = [];

        public string[] Names { get; private set; } = [];

        public ObjectKey?[] Keys { get; private set; } = [];

        public Dictionary<StableId, int> Index { get; } = [];

        public Dictionary<string, int> ByName { get; } = new(StringComparer.Ordinal);

        public Pose[] Poses { get; private set; } = [];

        private bool[] moved = [];
        private readonly List<int> movedList = [];
        private ulong mark;
        private Dictionary<string, List<int>>? kinds;
        private List<StableId>? players;
        private SceneModel? scene;

        /// <summary>Reads what moved since the last refresh; true when the structure changed and every place was found again.</summary>
        public bool Refresh(SceneModel scene)
        {
            if (!ReferenceEquals(scene, this.scene) || scene.Structure != Structure || scene.ObjectCount != Ids.Length)
            {
                Rebuild(scene);
                return true;
            }
            var written = scene.TransformsWrittenSince(mark);
            if (written is null)
            {
                for (var i = 0; i < Ids.Length; i++)
                {
                    Read(scene, i);
                }
            }
            else
            {
                foreach (var id in written)
                {
                    if (Index.TryGetValue(id, out var at))
                    {
                        Read(scene, at);
                    }
                }
            }
            mark = scene.TransformMark;
            return false;
        }

        private void Rebuild(SceneModel scene)
        {
            this.scene = scene;
            var order = Order(scene);
            Ids = [.. order];
            Names = new string[Ids.Length];
            Keys = new ObjectKey?[Ids.Length];
            Poses = new Pose[Ids.Length];
            moved = new bool[Ids.Length];
            movedList.Clear();
            Index.Clear();
            ByName.Clear();
            kinds = null;
            players = null;
            for (var i = 0; i < Ids.Length; i++)
            {
                var id = Ids[i];
                Index[id] = i;
                Names[i] = scene.NameOf(id) ?? "";
                ByName.TryAdd(Names[i], i);
                Keys[i] = scene.Key(id);
                Read(scene, i);
            }
            Structure = scene.Structure;
            mark = scene.TransformMark;
        }

        private void Read(SceneModel scene, int at) => Poses[at] = scene.TryGet<Transform>(Ids[at], out var t) ? PoseOf(t) : Pose.Initial;

        /// <summary>Moves the object at <paramref name="at"/>; it reaches the scene when the step's poses are written.</summary>
        public void Move(int at, in Pose pose)
        {
            Poses[at] = pose;
            if (!moved[at])
            {
                moved[at] = true;
                movedList.Add(at);
            }
        }

        public void Write(SceneModel scene)
        {
            if (movedList.Count == 0)
            {
                return;
            }
            if (ReferenceEquals(scene, this.scene))
            {
                foreach (var at in movedList)
                {
                    moved[at] = false;
                    if (Keys[at] is { } key)
                    {
                        scene.SetTransform(key, TransformOf(Poses[at]));
                    }
                }
                mark = scene.TransformMark;
            }
            movedList.Clear();
        }

        /// <summary>The places of the objects that are <paramref name="kind"/>: a component (name or label), a tag or a script (its name or file).</summary>
        public List<int> WithKind(string kind)
        {
            if (kinds is null)
            {
                kinds = new Dictionary<string, List<int>>(StringComparer.Ordinal);
                for (var i = 0; i < Ids.Length; i++)
                {
                    var found = new HashSet<string>(StringComparer.Ordinal);
                    foreach (var (name, label) in scene!.ComponentKinds(Ids[i]))
                    {
                        found.Add(Visual.KindKey(name));
                        found.Add(Visual.KindKey(label));
                    }
                    if (scene.TryGet<Tags>(Ids[i], out var tags))
                    {
                        found.UnionWith(tags.Values.Select(Visual.KindKey));
                    }
                    if (scene.TryGet<Scripts>(Ids[i], out var list))
                    {
                        foreach (var s in list.Items)
                        {
                            found.Add(Visual.KindKey(Path.GetFileNameWithoutExtension(s.Module)));
                            found.Add(Visual.KindKey(s.DisplayName()));
                        }
                    }
                    foreach (var key in found)
                    {
                        if (!kinds.TryGetValue(key, out var list2))
                        {
                            list2 = [];
                            kinds[key] = list2;
                        }
                        list2.Add(i);
                    }
                }
            }
            var wanted = Visual.KindKey(kind);
            return wanted.Length > 0 && kinds.TryGetValue(wanted, out var places) ? places : [];
        }

        /// <summary>The objects with Player Control, in hierarchy order.</summary>
        public List<StableId> Players => players ??= [.. Ids.Where(id => scene!.TryGet<PlayerControl>(id, out _))];
    }
}

using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;
using Bayleaf.Script.Sdk;

namespace Bayleaf.Script;

public sealed partial class ScriptHost
{
    /// <summary>Components physics reads when play starts, which scripts do not set.</summary>
    private static readonly string[] PhysicsComponents = ["rigidBody", "collider", "characterBody", "character"];

    /// <summary>The engine's side of the SDK's calls, for this host. Everything here runs on the script thread.</summary>
    private sealed class World(ScriptHost host) : IWorld
    {
        private ScriptHost Host { get; } = host;

        private View View => Host.view;

        public double Time => Host.Time;

        public int Current => Host.current;

        private Instance? Of(int slot) => slot >= 0 && slot < Host.instances.Count ? Host.instances[slot] : null;

        private StableId By(int slot) => Of(slot)?.Object ?? default;

        private static StableId Id(UInt128 id) => StableId.FromUInt128(id);

        private int? At(UInt128 id) => View.Index.TryGetValue(Id(id), out var at) ? at : null;

        private SceneObject ObjectAt(int at) => new(View.Ids[at].AsUInt128(), at, this);

        /// <summary>A name the calling script gave, checked: 1 to <paramref name="most"/> bytes and no control characters; a fault otherwise.</summary>
        private string Named(int slot, string? name, string what, int most = MaxMessageName)
        {
            if (string.IsNullOrEmpty(name) || Text.Bytes(name) > most || name.Any(char.IsControl))
            {
                throw Host.Broke(slot, $"{what} needs 1 to {most} bytes and no control characters");
            }
            return name;
        }

        private void Finite(int slot, Vector3 v, string what)
        {
            if (!(float.IsFinite(v.X) && float.IsFinite(v.Y) && float.IsFinite(v.Z)))
            {
                throw Host.Broke(slot, $"{what}: the numbers must be finite");
            }
        }

        public void Log(int slot, string text) => Host.AddLog(slot, text ?? "");

        public SceneObject Self(int slot) => Of(slot) is { } i ? new SceneObject(i.Object.AsUInt128(), i.View, this) : SceneObject.None;

        public bool Exists(UInt128 id) => At(id) is not null;

        public string? NameOf(UInt128 id) => At(id) is { } at ? View.Names[at] : null;

        public Pose? PoseOf(UInt128 id, int hint)
        {
            var ids = View.Ids;
            if ((uint)hint < (uint)ids.Length && ids[hint].AsUInt128() == id)
            {
                return View.Poses[hint];
            }
            return At(id) is { } at ? View.Poses[at] : null;
        }

        public bool SetPoseOf(int slot, UInt128 id, in Pose pose)
        {
            if (At(id) is not { } at)
            {
                return false;
            }
            if (!pose.IsFinite)
            {
                throw Host.Broke(slot, "the pose given is not finite");
            }
            View.Move(at, pose);
            return true;
        }

        public SceneObject Find(string name) => name is not null && View.ByName.TryGetValue(name, out var at) ? ObjectAt(at) : SceneObject.None;

        public SceneObject FindWith(string kind, int index)
        {
            var places = View.WithKind(kind ?? "");
            return index >= 0 && index < places.Count ? ObjectAt(places[index]) : SceneObject.None;
        }

        public int CountWith(string kind) => View.WithKind(kind ?? "").Count;

        /// <summary>The instance of a script named <paramref name="name"/> on the object, if one is there.</summary>
        private Instance? ScriptOn(StableId id, string name)
        {
            var key = Visual.KindKey(name);
            return key.Length == 0 ? null : Host.instances.FirstOrDefault(i => i.Object == id && i.Keys.Contains(key));
        }

        /// <summary>The component of the object a name or label means.</summary>
        private string? ComponentOn(StableId id, string name)
        {
            var key = Visual.KindKey(name);
            return Host.jobScene?.ComponentKinds(id).FirstOrDefault(c => Visual.KindKey(c.Name) == key || Visual.KindKey(c.Label) == key).Name;
        }

        public bool HasComponent(UInt128 id, string component)
        {
            var obj = Id(id);
            return At(id) is not null && (ScriptOn(obj, component ?? "") is not null || ComponentOn(obj, component ?? "") is not null);
        }

        public Vector3? Field(UInt128 id, string component, string field)
        {
            var obj = Id(id);
            if (At(id) is null || component is null || field is null)
            {
                return null;
            }
            if (ScriptOn(obj, component) is { Script: { } script } instance)
            {
                if (field.Length == 0)
                {
                    return Vector3.Zero;
                }
                return instance.Type.Fields.TryGetValue(field, out var info) ? ScriptType.Numbers(info.GetValue(script)) : null;
            }
            if (ComponentOn(obj, component) is not { } name || Host.jobScene is not { } scene)
            {
                return null;
            }
            if (field.Length == 0)
            {
                return Vector3.Zero;
            }
            return scene.Component(obj, name).TryGet(out var value, out _) && value.Get(field) is { } read ? FieldNumbers(read) : null;
        }

        public bool SetField(int slot, UInt128 id, string component, string field, Vector3 value)
        {
            var obj = Id(id);
            Finite(slot, value, "SetField");
            if (At(id) is null || string.IsNullOrEmpty(field))
            {
                return false;
            }
            if (ScriptOn(obj, component ?? "") is { } instance)
            {
                if (!instance.Type.Fields.ContainsKey(field) || !Host.Asked())
                {
                    return false;
                }
                Host.propertyWrites.Add((obj, Visual.KindKey(component!), field, value));
                return true;
            }
            if (ComponentOn(obj, component ?? "") is not { } name || PhysicsComponents.Contains(name) || Host.jobScene is not { } scene)
            {
                return false;
            }
            if (!scene.Component(obj, name).TryGet(out var current, out _) || current.Get(field) is not { } read || FieldNumbers(read) is null || !Host.Asked())
            {
                return false;
            }
            Host.requests.Add(new Request.SetField(By(slot), obj, name, field, value));
            return true;
        }

        public bool Send(int slot, UInt128? to, string name, float value, IReadOnlyList<EventValue> values)
        {
            Named(slot, name, "a message's name");
            if (!float.IsFinite(value))
            {
                throw Host.Broke(slot, "a message's value must be a finite number");
            }
            if (values.Count > Describer.MaxEventValues || values.Any(v => v.Kind == EventValueKind.Text && Text.Bytes(v.Text) > 4096))
            {
                throw Host.Broke(slot, $"an event brings at most {Describer.MaxEventValues} values, each text at most 4096 bytes");
            }
            if (to is { } target && At(target) is null)
            {
                return false;
            }
            if (!Host.SentOne())
            {
                return false;
            }
            Host.messages.Add(new Pending(to is { } t ? Id(t) : null, name, value, [.. values]));
            return true;
        }

        public bool Ask(int slot, UInt128 id, Ask ask, int count)
        {
            switch (ask)
            {
                case Sdk.Ask.PlaySound or Sdk.Ask.StopSound:
                    Host.NotYet(slot, "Sound");
                    return false;
                case Sdk.Ask.PlayParticles or Sdk.Ask.StopParticles:
                    Host.NotYet(slot, "Particles");
                    return false;
            }
            if (At(id) is null || !Host.Asked())
            {
                return false;
            }
            var by = By(slot);
            Host.requests.Add(ask switch
            {
                Sdk.Ask.Destroy => new Request.Destroy(by, Id(id)),
                Sdk.Ask.PlayKeyframes => new Request.Keyframes(by, Id(id), true),
                _ => new Request.Keyframes(by, Id(id), false),
            });
            return true;
        }

        public bool Animation(int slot, UInt128 id, AnimationAsk ask, string name, float value, bool flag)
        {
            if (ask is AnimationAsk.Play or AnimationAsk.Stop && !(float.IsFinite(value) && value >= 0f && value <= 60f))
            {
                throw Host.Broke(slot, "a fade is from 0 to 60 seconds");
            }
            if (ask == AnimationAsk.Number && !float.IsFinite(value))
            {
                throw Host.Broke(slot, "an animation number must be finite");
            }
            if (ask != AnimationAsk.Stop)
            {
                Named(slot, name, ask == AnimationAsk.Play ? "a clip's name" : "a parameter's name", 256);
            }
            if (At(id) is null || !Host.Asked())
            {
                return false;
            }
            AnimationAction action = ask switch
            {
                AnimationAsk.Play => new AnimationAction.Play(name, value, flag),
                AnimationAsk.Stop => new AnimationAction.Rest(value),
                AnimationAsk.Number => new AnimationAction.Number(name, value),
                AnimationAsk.Flag => new AnimationAction.Flag(name, flag),
                _ => new AnimationAction.Trigger(name),
            };
            Host.requests.Add(new Request.Animation(By(slot), Id(id), action));
            return true;
        }

        public string? AnimationState(UInt128 id) => Host.Animation.TryGetValue(Id(id), out var a) ? a.State : null;

        public Pose? Joint(UInt128 id, string joint)
        {
            if (!Host.Animation.TryGetValue(Id(id), out var a))
            {
                return null;
            }
            for (var i = 0; i < a.Names.Count && i < a.Joints.Count; i++)
            {
                if (a.Names[i] == joint)
                {
                    var (p, r, s) = a.Joints[i];
                    return new Pose(new Vector3d(p.X, p.Y, p.Z), r, s);
                }
            }
            return null;
        }

        public bool Interface(int slot, UInt128 id, InterfaceAsk ask, string element, string text, double value)
        {
            element ??= "";
            text ??= "";
            if (Text.Bytes(element) > 64 || text.Length > 0 && Text.Bytes(text) > 4096)
            {
                throw Host.Broke(slot, "an element's name has at most 64 bytes and its text at most 4096");
            }
            if (ask == InterfaceAsk.Value && !double.IsFinite(value))
            {
                throw Host.Broke(slot, "an element's value must be a finite number");
            }
            if (At(id) is null || !Host.Asked())
            {
                return false;
            }
            InterfaceChange change;
            switch (ask)
            {
                case InterfaceAsk.Text:
                    change = new InterfaceChange.Text(text);
                    break;
                case InterfaceAsk.Value:
                    change = new InterfaceChange.Value(value);
                    break;
                case InterfaceAsk.Show:
                    change = new InterfaceChange.Visible(value != 0);
                    break;
                case InterfaceAsk.Add:
                    var split = text.IndexOf('\n', StringComparison.Ordinal);
                    var (into, what) = split >= 0 ? (text[..split], text[(split + 1)..]) : ("", text);
                    if (Text.Bytes(what) > 512)
                    {
                        throw Host.Broke(slot, "what to add is a kind or a layout file of at most 512 bytes");
                    }
                    change = new InterfaceChange.Add(into, what);
                    break;
                default:
                    change = new InterfaceChange.Remove();
                    break;
            }
            Host.requests.Add(new Request.InterfaceEdit(By(slot), Id(id), element, change));
            return true;
        }

        public double? InterfaceValue(UInt128 id, string element) => Host.InterfaceValues.TryGetValue((Id(id), element ?? ""), out var v) ? v.Value : null;

        public string? InterfaceText(UInt128 id, string element) => Host.InterfaceValues.TryGetValue((Id(id), element ?? ""), out var v) ? v.Text : null;

        public bool Body(int slot, UInt128 id, BodyAsk ask, Vector3 value)
        {
            Finite(slot, value, ask.ToString());
            Host.NotYet(slot, "Physics");
            return false;
        }

        public Vector3? Velocity(UInt128 id)
        {
            Host.NotYet(Host.current, "Physics");
            return null;
        }

        public bool Grounded(UInt128 id)
        {
            Host.NotYet(Host.current, "Physics");
            return false;
        }

        public bool Spawn(int slot, string prefab, Vector3d at)
        {
            var path = Named(slot, prefab, "a prefab's path", 512);
            if (!at.IsFinite)
            {
                throw Host.Broke(slot, "a spawn's position must be finite");
            }
            if (!Host.Asked())
            {
                return false;
            }
            Host.requests.Add(new Request.Spawn(By(slot), path, new Double3(at.X, at.Y, at.Z)));
            return true;
        }

        public bool LoadScene(int slot, string path, bool add)
        {
            Named(slot, path, "a scene's path", 512);
            if (!Host.Asked())
            {
                return false;
            }
            Host.requests.Add(new Request.LoadScene(By(slot), path, add));
            return true;
        }

        public bool UnloadScene(int slot, string path)
        {
            Named(slot, path, "a scene's path", 512);
            if (!Host.Asked())
            {
                return false;
            }
            Host.requests.Add(new Request.UnloadScene(By(slot), path));
            return true;
        }

        public bool PlaySoundAt(int slot, string path, Vector3d at, float volume)
        {
            Host.NotYet(slot, "Sound");
            return false;
        }

        public RayHit? Raycast(int slot, Vector3d origin, Vector3 direction, double distance)
        {
            Host.NotYet(slot, "Rays (physics queries)");
            return null;
        }

        /// <summary>The player a state call names: the first player for none, or the object when it has Player Control.</summary>
        private StableId? Player(UInt128 id)
        {
            var players = View.Players;
            if (id == UInt128.Zero)
            {
                return players.Count > 0 ? players[0] : null;
            }
            var obj = Id(id);
            return players.Contains(obj) ? obj : null;
        }

        public double? State(int slot, StateScope scope, UInt128 player, string name)
        {
            Named(slot, name, "a value's name");
            return scope switch
            {
                StateScope.Game => Host.State.Get(ValueScope.Game, default, name),
                StateScope.Saved => Host.State.Get(ValueScope.Saved, default, name),
                _ => Player(player) is { } p ? Host.State.Get(ValueScope.Player, p, name) : null,
            };
        }

        public bool SetState(int slot, StateScope scope, UInt128 player, string name, double value)
        {
            Named(slot, name, "a value's name");
            return scope switch
            {
                StateScope.Game => Host.State.Set(ValueScope.Game, default, name, value),
                StateScope.Saved => Host.State.Set(ValueScope.Saved, default, name, value),
                _ => Player(player) is { } p && Host.State.Set(ValueScope.Player, p, name, value),
            };
        }

        public bool EndGame(bool won) => Host.State.End(won);

        public bool KeyDown(string key) => key is not null && Host.Held.Keys.Contains(key);

        public Vector2? Pointer => Host.Held.PointerAt;

        public bool MouseDown(int button) => button is >= 0 and < 3 && !Host.Held.OverInterface && ((Host.Held.Buttons >> button) & 1) == 1;

        public bool PointerOverInterface => Host.Held.OverInterface;

        public bool PadConnected => Host.Held.Pad is not null;

        public float PadAxis(int index) => Host.Held.Pad is { } pad && index >= 0 && index < pad.Axes.Count ? pad.Axes[index] : 0f;

        public bool PadButton(int index) => Host.Held.Pad is { } pad && index is >= 0 and < 32 && ((pad.Buttons >> index) & 1) == 1;

        public float Action(int slot, string name) => name is not null && Host.Input.Of(By(slot)) is { } maps ? maps.Value(name, Host.Held) : 0f;

        public bool SetInputMap(int slot, UInt128 id, string map)
        {
            var state = id == UInt128.Zero ? Host.Input.Project : Host.Input.Objects.GetValueOrDefault(Id(id));
            return state is not null && map is not null && state.SetActive(map);
        }

        /// <summary>A data file's field as play read it; <paramref name="index"/> an item of a list field, <paramref name="key"/> an entry of a map field.</summary>
        private PropertyValue? DataField(string file, string field, int index, string? key)
        {
            if (file is null || field is null || !Host.Data.TryGetValue(ScriptProject.Key(file), out var fields) || !fields.TryGetValue(field, out var resolved))
            {
                return null;
            }
            return (resolved.Shape.Kind, resolved.Value, index, key) switch
            {
                (ShapeKind.One, var v, < 0, null) => v,
                (ShapeKind.List, PropertyValue.List l, >= 0, null) when index < l.Items.Count => l.Items[index],
                (ShapeKind.Map, PropertyValue.Map m, < 0, { } k) => m.Items.GetValueOrDefault(k),
                _ => null,
            };
        }

        public DataValue? Data(string file, string field, int index, string? key) =>
            DataField(file, field, index, key) is { } v ? new DataValue(v.AsNumbers() ?? Vector3.Zero, v.AsText()) : null;

        public int DataCount(string file, string field)
        {
            if (file is null || field is null || !Host.Data.TryGetValue(ScriptProject.Key(file), out var fields) || !fields.TryGetValue(field, out var resolved))
            {
                return -1;
            }
            return resolved.Value switch
            {
                PropertyValue.List l => l.Items.Count,
                PropertyValue.Map m => m.Items.Count,
                _ => -1,
            };
        }

        public string? DataKey(string file, string field, int index)
        {
            if (file is null || field is null || !Host.Data.TryGetValue(ScriptProject.Key(file), out var fields) || !fields.TryGetValue(field, out var resolved))
            {
                return null;
            }
            return resolved.Value is PropertyValue.Map m && index >= 0 && index < m.Items.Count ? m.Items.Keys.ElementAt(index) : null;
        }
    }

    /// <summary>A component field's value as up to three numbers: a number, true/false as 1 or 0, or the first parts of a vector or colour; null for other fields.</summary>
    public static Vector3? FieldNumbers(JsonValue value)
    {
        switch (value)
        {
            case JsonNumber n:
                return new Vector3((float)n.ToDouble(), 0f, 0f);
            case JsonBool b:
                return new Vector3(b.Value ? 1f : 0f, 0f, 0f);
            case JsonArray a when a.Count is >= 2 and <= 4 && a.All(p => p is JsonNumber):
                var parts = a.Select(p => (float)((JsonNumber)p).ToDouble()).ToList();
                return new Vector3(parts[0], parts[1], parts.Count > 2 ? parts[2] : 0f);
            default:
                return null;
        }
    }

    /// <summary>A field set to <paramref name="numbers"/> the way <see cref="FieldNumbers"/> reads it, keeping its shape (a colour keeps its alpha); null for a field of another kind.</summary>
    public static JsonValue? WithField(JsonValue value, Vector3 numbers)
    {
        if (FieldNumbers(value) is null)
        {
            return null;
        }
        float[] given = [numbers.X, numbers.Y, numbers.Z];
        return value switch
        {
            JsonBool => JsonValue.From(numbers.X != 0f),
            JsonArray a => new JsonArray(a.Select((part, i) => i < 3 ? JsonNumber.Of(given[i]) : part)),
            _ => JsonNumber.Of(numbers.X),
        };
    }
}

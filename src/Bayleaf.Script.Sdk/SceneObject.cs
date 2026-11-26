using System.Numerics;

namespace Bayleaf.Script.Sdk;

/// <summary>
/// An object of the playing scene. It names the object by its id, so it stays good from step to step
/// and can be kept in a field; once the object is gone it reads as nothing and asks do nothing
/// (<see cref="Exists"/> says which). <c>default</c> is no object.
/// </summary>
/// <remarks>
/// Reads are as the object is now, with what scripts set earlier in this step. What changes other
/// objects or the scene (destroy, animation, interface, keyframes) is asked for and done after the
/// step, in the order asked; each returns false when it could not be asked (no object, or too many
/// asks in one call). Physics, sound and particles come with a later stage of the engine: those asks
/// return false and the console says so once.
/// </remarks>
public readonly struct SceneObject : IEquatable<SceneObject>
{
    internal SceneObject(UInt128 id, int hint = -1, IWorld? world = null) => (Id, Hint, World) = (id, hint, world);

    /// <summary>The engine that made it, so its calls need not look the engine up; null for one a script made.</summary>
    internal IWorld? World { get; }

    private IWorld Engine => World ?? Runtime.Need();

    internal UInt128 Id { get; }

    /// <summary>Where the engine last saw the object among its packed objects: checked first, so a kept object is found without a lookup.</summary>
    internal int Hint { get; }

    /// <summary>No object.</summary>
    public static SceneObject None => default;

    public bool IsNone => Id == UInt128.Zero;

    /// <summary>Whether the object is in the playing scene.</summary>
    public bool Exists => !IsNone && Engine.Exists(Id);

    /// <summary>Its name, or null once it is gone.</summary>
    public string? Name => IsNone ? null : Engine.NameOf(Id);

    /// <summary>Its pose relative to its parent, or null once it is gone.</summary>
    public Pose? Pose => IsNone ? null : Engine.PoseOf(Id, Hint);

    /// <summary>Moves the object; other scripts later in the step see it. False when it is gone or the pose is not finite.</summary>
    public bool SetPose(Pose pose) => !IsNone && Engine.SetPoseOf(Engine.Current, Id, pose);

    /// <summary>Sends the object's scripts a message, delivered after every script's step.</summary>
    public bool Send(string name, float value = 0f) => !IsNone && Engine.Send(Engine.Current, Id, name, value, []);

    /// <summary>Triggers an event on the object: a message with values (at most 16), delivered after the step.</summary>
    public bool Trigger(string name, params EventValue[] values) => !IsNone && Engine.Send(Engine.Current, Id, name, 0f, values);

    /// <summary>Deletes the object and its children after the step.</summary>
    public bool Destroy() => Ask(Sdk.Ask.Destroy);

    /// <summary>Whether the object has the component, by its name or label (<c>pointLight</c>, <c>Point Light</c>), or runs a script of that name.</summary>
    public bool Has(string component) => !IsNone && Engine.HasComponent(Id, component);

    /// <summary>
    /// A field of a component (by name or label; the field by its name in scene files) as up to three
    /// numbers: a number in X, true/false as 1 or 0, a vector or colour in all three. A script on the
    /// object, by the name the Inspector shows, is a component whose fields are its properties.
    /// </summary>
    public Vector3? Field(string component, string field) => IsNone ? null : Engine.Field(Id, component, field);

    /// <summary>Sets a field after the step, the way <see cref="Field"/> reads it (a colour keeps its alpha).</summary>
    public bool SetField(string component, string field, Vector3 value) => !IsNone && Engine.SetField(Engine.Current, Id, component, field, value);

    /// <summary>Sets a one-number or true/false field after the step.</summary>
    public bool SetField(string component, string field, float value) => SetField(component, field, new Vector3(value, 0f, 0f));

    /// <summary>Plays a clip on the whole skeleton of the object's model after the step, fading from what played.</summary>
    public bool PlayClip(string clip, float fade = 0.25f, bool loop = true) => Animation(AnimationAsk.Play, clip, fade, loop);

    /// <summary>Fades the model to its rest pose after the step.</summary>
    public bool StopClips(float fade = 0.25f) => Animation(AnimationAsk.Stop, "", fade, false);

    /// <summary>Sets a number parameter of the object's animation (mixes and animation states read them).</summary>
    public bool SetAnimationNumber(string name, float value) => Animation(AnimationAsk.Number, name, value, false);

    /// <summary>Sets a true/false parameter of the object's animation.</summary>
    public bool SetAnimationFlag(string name, bool value) => Animation(AnimationAsk.Flag, name, 0f, value);

    /// <summary>Sets a trigger, used up by the first transition it lets through.</summary>
    public bool TriggerAnimation(string name) => Animation(AnimationAsk.Trigger, name, 0f, true);

    /// <summary>The animation state the object is in (as of the last step), or null when it runs none.</summary>
    public string? AnimationState => IsNone ? null : Engine.AnimationState(Id);

    /// <summary>Where a joint of the object's model is in the world as the last step posed it, or null.</summary>
    public Pose? Joint(string name) => IsNone ? null : Engine.Joint(Id, name);

    /// <summary>Sets the text of an element of the object's Interface after the step.</summary>
    public bool SetInterfaceText(string element, string text) => Interface(InterfaceAsk.Text, element, text, 0);

    /// <summary>Sets an element's value after the step: a toggle's 0 or 1, a slider's number, a progress bar's fill.</summary>
    public bool SetInterfaceValue(string element, double value) => Interface(InterfaceAsk.Value, element, "", value);

    /// <summary>Shows or hides an element after the step, or the whole interface for an empty name.</summary>
    public bool ShowInterface(bool visible, string element = "") => Interface(InterfaceAsk.Show, element, "", visible ? 1 : 0);

    /// <summary>Adds an element after the step: <paramref name="what"/> is a kind (<c>button</c>, <c>text</c>…) or a layout file (<c>.bui</c>) used as a part.</summary>
    public bool AddInterfaceElement(string name, string what, string into = "") => Interface(InterfaceAsk.Add, name, into + "\n" + what, 0);

    /// <summary>Removes an element, with what it holds, after the step.</summary>
    public bool RemoveInterfaceElement(string name) => Interface(InterfaceAsk.Remove, name, "", 0);

    /// <summary>An element's value as it shows, or null when there is no such element.</summary>
    public double? InterfaceValue(string element) => IsNone ? null : Engine.InterfaceValue(Id, element);

    /// <summary>An element's text as it shows (what the player typed, too), or null.</summary>
    public string? InterfaceText(string element) => IsNone ? null : Engine.InterfaceText(Id, element);

    /// <summary>Plays the object's Keyframe Animation from its start, after the step.</summary>
    public bool PlayKeyframes() => Ask(Sdk.Ask.PlayKeyframes);

    /// <summary>Stops the object's Keyframe Animation where it is, after the step.</summary>
    public bool StopKeyframes() => Ask(Sdk.Ask.StopKeyframes);

    /// <summary>Makes a map active in the object's own Input Map.</summary>
    public bool SetInputMap(string map) => !IsNone && Engine.SetInputMap(Engine.Current, Id, map);

    /// <summary>Plays the object's Audio Source after the step. Sound comes with a later stage: false for now.</summary>
    public bool PlaySound() => Ask(Sdk.Ask.PlaySound);

    /// <summary>Stops the sounds the object plays. Sound comes with a later stage: false for now.</summary>
    public bool StopSound() => Ask(Sdk.Ask.StopSound);

    /// <summary>Starts the object's Particle Emitter. Particles come with a later stage: false for now.</summary>
    public bool PlayParticles() => Ask(Sdk.Ask.PlayParticles);

    /// <summary>Stops the object's Particle Emitter. Particles come with a later stage: false for now.</summary>
    public bool StopParticles() => Ask(Sdk.Ask.StopParticles);

    /// <summary>Starts this many particles at once. Particles come with a later stage: false for now.</summary>
    public bool BurstParticles(int count) => !IsNone && count > 0 && Engine.Ask(Engine.Current, Id, Sdk.Ask.PlayParticles, count);

    /// <summary>Pushes the object's dynamic physics body by an impulse. Physics comes with a later stage: false for now.</summary>
    public bool Push(Vector3 impulse) => Body(BodyAsk.Push, impulse);

    /// <summary>Sets the body's velocity. Physics comes with a later stage: false for now.</summary>
    public bool SetVelocity(Vector3 velocity) => Body(BodyAsk.Velocity, velocity);

    /// <summary>Sets the Character's walking velocity. Physics comes with a later stage: false for now.</summary>
    public bool Walk(Vector3 velocity) => Body(BodyAsk.Walk, velocity);

    /// <summary>Makes the Character jump. Physics comes with a later stage: false for now.</summary>
    public bool Jump(float speed) => Body(BodyAsk.Jump, new Vector3(speed, 0f, 0f));

    /// <summary>The body's velocity; null without physics (a later stage).</summary>
    public Vector3? Velocity => IsNone ? null : Engine.Velocity(Id);

    /// <summary>Whether the Character stands on something; false without physics (a later stage).</summary>
    public bool IsGrounded => !IsNone && Engine.Grounded(Id);

    private bool Ask(Ask ask) => !IsNone && Engine.Ask(Engine.Current, Id, ask, 0);

    private bool Animation(AnimationAsk ask, string name, float value, bool flag) => !IsNone && Engine.Animation(Engine.Current, Id, ask, name, value, flag);

    private bool Interface(InterfaceAsk ask, string element, string text, double value) => !IsNone && Engine.Interface(Engine.Current, Id, ask, element, text, value);

    private bool Body(BodyAsk ask, Vector3 value) => !IsNone && Engine.Body(Engine.Current, Id, ask, value);

    public static bool operator ==(SceneObject a, SceneObject b) => a.Id == b.Id;

    public static bool operator !=(SceneObject a, SceneObject b) => a.Id != b.Id;

    public bool Equals(SceneObject other) => Id == other.Id;

    public override bool Equals(object? obj) => obj is SceneObject other && Equals(other);

    public override int GetHashCode() => Id.GetHashCode();

    public override string ToString() => IsNone ? "none" : Id.ToString("x32", System.Globalization.CultureInfo.InvariantCulture);
}

/// <summary>What a ray hit (see <see cref="Game.Raycast"/>).</summary>
public readonly record struct RayHit(SceneObject Hit, double Distance, Vector3d Point);

# C# scripts

A game script of the C# engine is a C# class in a `.cs` file of the project, written against
`Bayleaf.Script.Sdk` (`src/Bayleaf.Script.Sdk`). It can do what `docs/script-abi.md` says a
WebAssembly script of the Rust engine can do, in a C# shape: the engine calls the script's methods,
the script reads and changes its object through plain values, and what it asks of the scene is done
after the step. Visual scripts will compile to the same C# (a later step of stage 4 in
`docs/csharp-redesign.md`); until then the C# engine does not run them and says so.

## A script

```csharp
public class Spinner : GameScript, IAdvanceMany<Spinner>
{
    [ScriptProperty(Hint = "Degrees per second")]
    private float speed = 90f;

    protected override void Begin() => Log("spinner: started");

    // Every spinning object in one call: each one's script and pose, by the same index.
    public static void AdvanceMany(Many<Spinner> objects, float seconds)
    {
        for (var i = 0; i < objects.Count; i++)
        {
            ref var rotation = ref objects.Poses[i].Rotation;
            rotation.Y = (rotation.Y + objects.Scripts[i].speed * seconds) % 360f;
        }
    }
}
```

- **One script per file.** A file holds one public, non-generic class that derives from `GameScript`
  and has a public constructor that takes nothing; the scene's `scripts` component names the file
  (`{"module": "scripts/spinner.cs"}`), and the Inspector shows it under the file's name in title
  case ("Spinner"). Helper classes may live in other files: every `.cs` file of the project folder
  (hidden, `bin`, `obj`, `target` and `.build` folders left out) compiles into one assembly. Files
  start with `System`, `System.Collections.Generic`, `System.Linq`, `System.Numerics` and
  `Bayleaf.Script.Sdk` in scope.
- **The calls.** `PropertiesChanged()` when the properties are set (before `Prepare`, and again when
  the Inspector or another script changes one); `Prepare()` once, and after every script loaded with
  it (a scene when play starts, a placed prefab, an added scene) prepared, `Begin()` once; every step
  `Advance(float seconds)`, or for a script that implements `IAdvanceMany<TSelf>` one
  `AdvanceMany(Many<TSelf>, float)` for all of its objects; `Receive(in Message)` for each message
  after the step. Scripts that step per object run first, in the order they loaded (object by object,
  each object's scripts in their order), then each script type that steps many. These are our names,
  not another engine's.
- **Properties are fields.** A field marked `[ScriptProperty]` (private is fine) is a property: a
  `float` or `double` is a number, `int` a whole number, `bool` true/false, `Vector3` a vector (a
  colour with `Color = true`), `string` text (or a file of the project with `File = "prefab"`,
  `"sound"`, `"material"` or `"data"`) and `SceneObject` an object, by its name. Its name in scene
  files is the field's name starting in lower case unless the attribute gives one; its default is the
  field's initial value, which must be a constant (or `new Vector3(...)` of constants). `Label`,
  `Hint`, `Low` and `High` (a range), `Slider`, `ReadOnly` and `Choices` (`"Walk|Run"`) lay it out in
  the Inspector, as `script-abi.md` describes. Lists and maps as properties are not in the C#
  engine yet (C# scripts keep their own collections in fields).
- **Where Add Component lists it:** `[ScriptMenu("Light/Flicker")]` on the class. **Events it
  triggers:** `[ScriptEvent("Damaged", "amount:number", "by:object")]`, as many as it has.

What a script declares is read from its source by the compiler, so describing it (the Inspector,
`asset.info`) runs no script code and needs no consent. A property of a type scripts cannot have, a
default that is not a constant, a second script in a file or a missing constructor is a problem of
that script: `asset.info` lists it, and play mode does not run the script.

## What a script can do

| | |
|---|---|
| Its object | `Self` (a `SceneObject`), `Pose` (position in double precision, rotation in degrees, scale: a reference into the engine's packed poses, so `Pose.Rotation.Y += 90 * seconds` turns it; `var pose = Pose; … Pose = pose;` works too), `Log(text)`, `Time` |
| Other objects | `Game.Find(name)`, `Game.FindWith(kind, index)`, `Game.CountWith(kind)`; a `SceneObject` stays good from step to step and reads as nothing once its object is gone: `Name`, `Pose`, `SetPose`, `Field(component, field)` and `SetField(...)` (a script on the object is a component whose fields are its properties), `Has(component)`, `Send(name, value)`, `Trigger(name, values…)`, `Destroy()` |
| Animation | `PlayClip`, `StopClips`, `SetAnimationNumber`, `SetAnimationFlag`, `TriggerAnimation`, `AnimationState`, `Joint(name)`; `PlayKeyframes`, `StopKeyframes` |
| Interface | `SetInterfaceText`, `SetInterfaceValue`, `ShowInterface`, `AddInterfaceElement`, `RemoveInterfaceElement`, `InterfaceValue`, `InterfaceText`; the object's scripts hear `pressed:<name>` and `changed:<name>` |
| The game | `Game.Spawn(prefab, position)`, `Game.LoadScene(path, add)`, `Game.UnloadScene(path)`, `Game.SendToAll`, `Game.TriggerOnAll`, `Game.Value`/`SetValue`, `Game.PlayerValue`/`SetPlayerValue`, `Game.SavedValue`/`SetSavedValue`, `Game.End(won)` (every script hears `gameWon` or `gameLost`), `Game.Data(path)` (a data file as play read it: `Number`, `Whole`, `Flag`, `Vector`, `Text`, `Count`, `NumberAt`, `TextAt`, `KeyAt`, `NumberFor`, `TextFor`) |
| Input | `Controls.KeyDown`, `PointerPosition`, `MouseDown`, `PointerOverInterface`, `PadConnected`, `PadAxis`, `PadButton`, `Action(name)` and `ActionHeld(name)` (the object's own Input Map first, then the project's), `SetInputMap`, and `SceneObject.SetInputMap` |
| Messages | `Receive(in Message)`: `Name`, `Value`, `Is(name)`, and an event's `Values` (`EventValue`: number, whole, flag, vector, text or object) |
| Not yet | Physics (`Push`, `SetVelocity`, `Walk`, `Jump`, `Velocity`, `IsGrounded`, `Game.Raycast`), sound (`PlaySound`, `StopSound`, `Game.PlaySoundAt`) and particles (`PlayParticles`, `StopParticles`, `BurstParticles`) come with stage 5: they return false (or nothing) and the console says so once per play. Subscribing to another component's events is not in the C# engine yet |

What changes other objects or the scene is asked for and done after the step, in the order asked,
as for WebAssembly scripts: at most 64 asks and 64 messages per call (per object for a script that
steps many). Poses changed during a step reach the scene once per object when the step ends (only
those a script reached and changed are written); later scripts of the step see them. A pose that is
not finite is not written: the object stays where it was and the console says so. Breaking a rule (a message name of more than 64 bytes, a number that is
not finite, a fade outside 0 to 60 seconds) stops the script with the reason, even if its code
catches the exception the call throws. An exception a script throws stops that script (and, in
`AdvanceMany`, the script on every object it steps) with its reason in the console; the other scripts
keep running.

`GameScript.Time`, `Pose` and a `SceneObject` the engine gave reach the engine directly. `Game` and
`Controls` are static and find the engine through the script thread, which costs a little more per
call: inside `Advance`, prefer `Time` to `Game.Time`.

## Safety and limits

A C# script runs in the engine's process with no sandbox (`docs/csharp-redesign.md`, "Scripts"):

- **Consent.** A project's scripts run only once the person at the editor allowed them. The session
  keeps the consent per project folder (`Session.ScriptsAllowed`); `project.scripts.allow` is the
  command the editor's question and its Project > Allow Scripts menu give the answer through, and
  the CLI takes `--allow-scripts` for `run` and `batch`. The C# editor (stage 6) is to ask when a
  project is opened for the first time and remember the answer in the person's own settings, as a
  code editor remembers the folders it trusts. Agents cannot give it (the
  command is disabled under automation), so an agent's `play.start` runs the scene without its
  scripts, says so in its result (`notRun`) and in the console, and plays on.
- **The watchdog.** The scripts run on a script thread of their own; play mode waits for each step's
  scripts at most `Session.ScriptTimeLimit` (2 seconds by default) and stops play mode when they take
  longer, putting the scene back and saying which script was running. .NET cannot stop a thread from
  outside, so a script stuck in a loop keeps that thread busy until the program ends; nothing else
  uses the scene or host it holds. A script that recurses without end ends the program, as any .NET
  code does.
- **Compiling runs nothing.** The engine compiles with Roslyn (`Microsoft.CodeAnalysis.CSharp`)
  against .NET's own assemblies and the SDK only, with no analyzers or source generators of the
  project, so compiling (`script.build`, `asset.info`) is safe without consent.

## Compiling and reloading

Play mode compiles the project's scripts when it starts (again only when a file changed since) and
loads them into a collectible `AssemblyLoadContext`. While it plays it looks for changed files about
twice a second of play time: changed scripts that compile are loaded anew and every script starts
again (made again, its properties set, `Prepare` and `Begin`), keeping what play changed in the scene;
the old load context is let go. Scripts that changed but do not compile are reported once, with the
compiler's messages, and the running ones keep running. `script.build` compiles them on request and
gives the problems with file, line and column.

`script.new` (and `asset.create` with kind `script`) writes a script from a template:
`<folder>/<name>.cs`, the file name in lower case with `_` (`Turn Table` → `turn_table.cs`), the class
in title case (`TurnTable`). A project needs no `.csproj` for its scripts; one that references
`Bayleaf.Script.Sdk` gives an IDE completion, as `scripts/Bayleaf.Examples.Scripts.csproj` does for the
example scripts in `scripts/` (the ports of `rust/scripts`).

In an exported game the scripts will compile into the player ahead of time (stage 7), with the same
calls.

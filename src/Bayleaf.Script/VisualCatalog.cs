using System.Numerics;

using Bayleaf.Graph;

namespace Bayleaf.Script;

/// <summary>
/// Visual scripts (domain <c>script</c>, files <c>.bscript</c>): their node kinds, the pins a node's
/// settings give it, and the upgrade of older files. Event nodes (On Start, On Update) begin chains of
/// actions joined by flow links; data nodes are pure, computed where an action needs them. Visual
/// scripts will compile to C# (docs/csharp-redesign.md, stage 4); this port has no compiler yet.
/// </summary>
/// <remarks>
/// Every value the catalog is built from is in this file, above <see cref="Catalog"/>, so it is set
/// before the catalog is (static fields of one type in several files are set in no fixed order).
/// </remarks>
public static partial class Visual
{
    private static readonly PinType F = PinType.Number;
    private static readonly PinType V = PinType.Vec3;
    private static readonly PinType B = PinType.Bool;
    private static readonly PinType A = PinType.Any;

    private static Vector3 Xyz(float x, float y, float z) => new(x, y, z);

    private static PinDef Pin(string name, string label, PinType type) => new(name, label, type);

    private static PinDef With(string name, string label, PinType type, PinDefault value) => new(name, label, type, value);

    private static PinDef Number(string name, string label, PinType type, float value) => new(name, label, type, new PinDefault.Number(value));

    private static PinDef Vector(string name, string label, Vector3 value) => new(name, label, PinType.Vec3, new PinDefault.Vec3(value));

    private static PinDef TextPin(string name, string label, string value) => new(name, label, PinType.Text, new PinDefault.Text(value));

    private static SettingDef Setting(string name, string label, SettingKind kind) => new(name, label, kind);

    private static NodeDef Def(string kind, string label, string category, string description, IReadOnlyList<PinDef> inputs, IReadOnlyList<PinDef> outputs, IReadOnlyList<SettingDef> settings, bool unique) =>
        new(kind, label, category, description, inputs, outputs, settings, unique);

    private static NodeDef Node(string kind, string label, string category, string description, IReadOnlyList<PinDef> inputs, IReadOnlyList<PinDef> outputs) =>
        new(kind, label, category, description, inputs, outputs, [], false);

    private static NodeDef ActionNode(string kind, string label, string description, IReadOnlyList<PinDef> inputs) =>
        new(kind, label, "Actions", description, inputs, [Then], [], false);

    private static readonly IReadOnlyList<PinDef> TextOut = [Pin("result", "Result", PinType.Text)];

    internal static readonly PinDef Run = Pin("run", "Run", PinType.Flow);
    internal static readonly PinDef Then = Pin("then", "Then", PinType.Flow);
    private static readonly IReadOnlyList<PinDef> MathOut = [Pin("result", "Result", A)];
    private static readonly SettingDef MessageName = Setting("message", "Message", new SettingKind.Name("hit"));
    private static readonly IReadOnlyList<PinDef> ContactOut = [Then, Pin("other", "Other Object", PinType.SceneObject)];

    /// <summary>An event node for a physics contact: what it runs after, and the other object.</summary>
    private static NodeDef Contact(string kind, string label, string description) => new(kind, label, "Physics", description, [], ContactOut, [], false);

    private static readonly SettingDef NumberVariable = Setting("name", "Name", new SettingKind.Name("count"));
    private static readonly SettingDef VectorVariable = Setting("name", "Name", new SettingKind.Name("target"));

    /// <summary>An action of an Input Map: <c>Jump</c> in the active map, or <c>Player/Jump</c>, live only while Player is active.</summary>
    private static readonly SettingDef ActionName = Setting("action", "Action", new SettingKind.Text("Jump"));
    private static readonly SettingDef StateName = Setting("name", "Name", new SettingKind.Name("score"));

    /// <summary>
    /// Whose value: the game's, a player's (the one named in Player, else the first), this object's own
    /// when it is a player, or a saved one that lasts from one play to the next.
    /// </summary>
    private static readonly SettingDef StateScope = Setting("scope", "Scope", new SettingKind.Choice(["Game", "Player", "This Object", "Saved"]));
    private static readonly SettingDef StatePlayer = Setting("player", "Player", new SettingKind.Text(""));
    private static readonly SettingDef FlagVariable = Setting("name", "Name", new SettingKind.Name("done"));

    /// <summary>A scene file of the project, relative to its folder.</summary>
    private static readonly SettingDef SceneFile = Setting("scene", "Scene File", new SettingKind.Text(""));
    private static readonly SettingDef TextVariable = Setting("name", "Name", new SettingKind.Name("label"));
    private static readonly SettingDef ObjectVariable = Setting("name", "Name", new SettingKind.Name("thing"));
    private static readonly SettingDef ColorVariable = Setting("name", "Name", new SettingKind.Name("tint"));
    private static readonly SettingDef WholeVariable = Setting("name", "Name", new SettingKind.Name("total"));

    /// <summary>Most bytes a text variable keeps; longer text is cut at a character.</summary>
    public const int TextVariableBytes = 1024;

    /// <summary>
    /// The object an action works on or a value is read from, from a search or This Object; when it is
    /// not linked, the node uses the object named in Object Name (or its own default).
    /// </summary>
    internal static readonly PinDef Target = Pin("target", "Object", PinType.SceneObject);

    /// <summary>The property a Property or Set Property node names, declared in the script's panel.</summary>
    private static readonly SettingDef PropertyName = Setting("name", "Property", new SettingKind.Name(""));

    /// <summary>A Property node of one type: the property's value as an output.</summary>
    private static NodeDef GetProperty(string kind, string label, string description, PinType type) =>
        new(kind, label, "Properties", description, [], [Pin("value", "Value", type)], [PropertyName], false);

    /// <summary>A Set Property node of one type: sets the property to its Value input.</summary>
    private static NodeDef SetProperty(string kind, string label, string description, PinDef value) =>
        new(kind, label, "Properties", description, [Run, value], [Then], [PropertyName], false);

    /// <summary>
    /// The function a Function, Return or Call Function node belongs to, and its pins (<c>name:type</c>),
    /// kept in step with the function by <c>graph.function.set</c>.
    /// </summary>
    private static readonly IReadOnlyList<SettingDef> FunctionSettings =
    [
        Setting("function", "Function", new SettingKind.Name("")),
        Setting("inputs", "Inputs", new SettingKind.Names()),
        Setting("outputs", "Outputs", new SettingKind.Names()),
    ];

    /// <summary>
    /// The event an On Event or Trigger Event node names, and its values' pins (<c>name:type</c>), kept in
    /// step with the graph's declaration by <c>graph.event.set</c>.
    /// </summary>
    private static readonly IReadOnlyList<SettingDef> EventSettings = [Setting("event", "Event", new SettingKind.Name("")), Setting("params", "Values", new SettingKind.Names())];

    /// <summary>A component input: a script on an object, from a Get Component that names a script or This Component.</summary>
    internal static readonly PinDef ComponentIn = Pin("component", "Component", PinType.Component);

    /// <summary>
    /// The script whose event a node names (its name as the Inspector shows it), the event, and its
    /// values' pins (<c>name:type</c>). The Graph panel picks the event from the script's own list.
    /// </summary>
    private static readonly IReadOnlyList<SettingDef> ComponentEventSettings = [Setting("script", "Script", new SettingKind.Text("")), EventSettings[0], EventSettings[1]];

    /// <summary>
    /// The list a list node works on, one the script declares, and the type code of what it holds
    /// (<c>["number"]</c>), kept in step with the declaration by <c>graph.variable.set</c> and <c>graph.property.set</c>.
    /// </summary>
    private static readonly IReadOnlyList<SettingDef> ListSettings = [Setting("name", "List", new SettingKind.Name("")), Setting("item", "Holds", new SettingKind.Names())];

    /// <summary>The map a map node works on, and what it holds, as for a list.</summary>
    private static readonly IReadOnlyList<SettingDef> MapSettings = [Setting("name", "Map", new SettingKind.Name("")), Setting("item", "Holds", new SettingKind.Names())];

    /// <summary>A list index, from 0.</summary>
    private static readonly PinDef IndexPin = With("index", "Index", PinType.WholeNumber, new PinDefault.WholeNumber(0));

    /// <summary>A map key.</summary>
    private static readonly PinDef KeyPin = TextPin("key", "Key", "");

    /// <summary>The list a list node works on, as a pin (of numbers here; a node's own type is its <c>item</c>).</summary>
    private static readonly PinDef ListPin = Pin("list", "List", PinType.List(VariableType.Number));

    /// <summary>The map a map node works on, as a pin.</summary>
    private static readonly PinDef MapPin = Pin("map", "Map", PinType.Map(VariableType.Text, VariableType.Number));

    /// <summary>The value a list or map node puts in, and gives.</summary>
    private static readonly PinDef ValueIn = Number("value", "Value", F, 0f);
    private static readonly PinDef ValueOut = Pin("value", "Value", F);

    private static NodeDef ListNode(string kind, string label, string description, IReadOnlyList<PinDef> inputs, IReadOnlyList<PinDef> outputs) =>
        new(kind, label, "Lists", description, inputs, outputs, ListSettings, false);

    private static NodeDef MapNode(string kind, string label, string description, IReadOnlyList<PinDef> inputs, IReadOnlyList<PinDef> outputs) =>
        new(kind, label, "Maps", description, inputs, outputs, MapSettings, false);

    private static readonly SettingDef ObjectName = Setting("object", "Object Name", new SettingKind.Text(""));

    /// <summary>The parameter an animation node sets.</summary>
    private static readonly SettingDef AnimationParameter = Setting("parameter", "Parameter", new SettingKind.Name("speed"));

    /// <summary>The interface element a node works on, by its name.</summary>
    private static readonly SettingDef ElementName = Setting("element", "Element", new SettingKind.Text("Start"));

    /// <summary>What a search looks for: a component (Point Light, Character), a tag, or a script (by its name).</summary>
    private static readonly SettingDef SearchKind = Setting("kind", "Component, Tag or Script", new SettingKind.Text("Point Light"));

    // The components visual scripts read and set field by field: Get Component, Try Get Component and
    // Set Component show the fields of the component named in their Component setting as pins.

    /// <summary>The Component setting of Get Component, Try Get Component and Set Component.</summary>
    internal static readonly SettingDef ComponentSetting = Setting("component", "Component", new SettingKind.Text("Point Light"));

    /// <summary>The properties of the script a node names, kept by the node.</summary>
    internal static readonly SettingDef FieldsSetting = Setting("fields", "Fields", new SettingKind.Names());

    /// <summary>The data shape whose fields a Data File node shows (<c>.bshape</c>), chosen in the editor.</summary>
    internal static readonly SettingDef DataShape = Setting("shape", "Shape", new SettingKind.Text(""));

    /// <summary>The data file a Data File node reads, by its path in the project (or linked text).</summary>
    internal static readonly PinDef DataFile = Pin("file", "File", PinType.Text);

    internal const string DataText = "The fields of a data file (.bdata) as it was saved when play started, each the shape's default where the file sets none. The fields are the chosen shape's; lists and maps have no pin.";

    internal const string GetText = "The fields of a component on the linked object (else this object), as the step began. A missing component reads as zeros; Try Get Component says whether it was found.";

    internal const string TryGetText = "The fields of a component on the linked object (else this object), as the step began, and whether the object has it.";

    internal const string SetText = "Sets the linked fields of a component on the linked object (else this object), after this step. Fields without a link keep their values.";

    /// <summary>The node kinds of visual scripts.</summary>
    public static readonly Catalog Catalog = new()
    {
        Domain = "script",
        Label = "Visual Script",
        Extension = "bscript",
        Starter = [("start", new Vector2(40f, 40f)), ("update", new Vector2(40f, 200f))],
        Variant = Variant,
        Upgrade = Upgrade,
        Fractional = ["sine", "squareRoot", "lerp"],
        Nodes =
        [
            Node("init", "On Init", "Events", "Runs once when the object is ready, before any script's On Start: every script of the scene runs On Init first.", [], [Then]),
            Node("start", "On Start", "Events", "Runs once when play mode starts, after every script ran On Init.", [], [Then]),
            Node("update", "On Update", "Events", "Runs every simulation step.", [], [Then, Pin("dt", "Step Seconds", F)]),
            Def("message", "On Message", "Events", "Runs when a script sends this message to this object, or to every object. Value is the number sent with it.", [], [Then, Pin("value", "Value", F)], [MessageName], false),
            Contact("triggerEntered", "On Trigger Entered", "Runs when another collider comes into this object's trigger, or this object comes into another's (needs physics)."),
            Contact("triggerLeft", "On Trigger Left", "Runs when another collider leaves this object's trigger, or this object leaves another's (needs physics)."),
            Contact("collisionStarted", "On Collision Started", "Runs when this object's collider starts touching another (needs physics)."),
            Contact("collisionEnded", "On Collision Ended", "Runs when this object's collider stops touching another (needs physics)."),
            Def("sendMessage", "Send Message", "Actions", "Sends a message and a number to the scripts of the linked object, else the first object named in Object Name, else every object. It arrives after this step.", [Run, Target, Number("value", "Value", F, 0f)], [Then], [MessageName, ObjectName], false),
            Def("action", "Action", "Inputs", "An input action's value by its kind (a trigger 1 in the step it goes down, held 1 while down, value how far it is pressed), and whether it is down. Name it Jump for the active map, or Player/Jump; actions of maps that are not active read 0.", [], [Pin("value", "Value", F), Pin("held", "Held", B)], [ActionName], false),
            Def("actionPressed", "On Action Pressed", "Events", "Runs once in the step an input action goes down, while its map is active.", [], [Then], [ActionName], false),
            Def("keyPressed", "On Key Pressed", "Events", "Runs once in the step when the key goes down (names like W, Space, ArrowUp).", [], [Then], [Setting("key", "Key", new SettingKind.Name("Space"))], false),
            Def("timer", "On Timer", "Events", "Runs every given number of seconds of play.", [], [Then], [Setting("seconds", "Seconds", new SettingKind.Number(1f))], false),
            Def("setVectorVariable", "Set Vector Variable", "Variables", "Keeps a vector under a name, for this script on this object. It starts where the Variables panel says, or at 0, 0, 0.", [Run, Vector("value", "Value", Xyz(0f, 0f, 0f))], [Then], [VectorVariable], false),
            Def("vectorVariable", "Vector Variable", "Variables", "The vector Set Vector Variable last kept under this name, or 0, 0, 0.", [], [Pin("value", "Value", V)], [VectorVariable], false),
            Def("setFlag", "Set True/False Variable", "Variables", "Keeps true or false under a name, for this script on this object. It starts as the Variables panel says, or false.", [Run, With("value", "Value", B, new PinDefault.Bool(false))], [Then], [FlagVariable], false),
            Def("flag", "True/False Variable", "Variables", "What Set True/False Variable last kept under this name, or false.", [], [Pin("value", "Value", B)], [FlagVariable], false),
            Def("setWholeVariable", "Set Whole Number Variable", "Variables", "Keeps a whole number under a name, for this script on this object. It starts where the Variables panel says, or at 0.", [Run, With("value", "Value", PinType.WholeNumber, new PinDefault.WholeNumber(0))], [Then], [WholeVariable], false),
            Def("wholeVariable", "Whole Number Variable", "Variables", "The whole number Set Whole Number Variable last kept under this name, or what it starts at.", [], [Pin("value", "Value", PinType.WholeNumber)], [WholeVariable], false),
            Def("setTextVariable", "Set Text Variable", "Variables", "Keeps text under a name, for this script on this object (up to 1024 bytes; longer text is cut). It starts as the Variables panel says, or empty.", [Run, TextPin("value", "Value", "")], [Then], [TextVariable], false),
            Def("textVariable", "Text Variable", "Variables", "The text Set Text Variable last kept under this name, or what it starts as.", [], [Pin("value", "Value", PinType.Text)], [TextVariable], false),
            Def("setObjectVariable", "Set Object Variable", "Variables", "Keeps an object under a name, for this script on this object, from step to step. It starts as none.", [Run, Pin("value", "Value", PinType.SceneObject)], [Then], [ObjectVariable], false),
            Def("objectVariable", "Object Variable", "Variables", "The object Set Object Variable last kept under this name; none (Found false) before that, or once the object is gone.", [], [Pin("value", "Value", PinType.SceneObject), Pin("found", "Found", B)], [ObjectVariable], false),
            Def("setColorVariable", "Set Color Variable", "Variables", "Keeps a colour under a name, for this script on this object. It starts as the Variables panel says, or white.", [Run, With("value", "Value", PinType.Color, new PinDefault.Color(Xyz(1f, 1f, 1f)))], [Then], [ColorVariable], false),
            Def("colorVariable", "Color Variable", "Variables", "The colour Set Color Variable last kept under this name, or what it starts as.", [], [Pin("value", "Value", PinType.Color)], [ColorVariable], false),
            Def("setVariable", "Set Variable", "Variables", "Keeps a number under a name, for this script on this object. It starts where the Variables panel says, or at 0.", [Run, Number("value", "Value", F, 0f)], [Then], [NumberVariable], false),
            Def("variable", "Variable", "Variables", "The number Set Variable last kept under this name, or 0.", [], [Pin("value", "Value", F)], [NumberVariable], false),
            Def("log", "Log", "Actions", "Writes the message, and the value if one is linked, to the console.", [Run, TextPin("text", "Text", ""), Pin("value", "Value", F)], [Then], [Setting("message", "Message", new SettingKind.Text("Hello"))], false),
            ActionNode("setPosition", "Set Position", "Moves the object to a position (relative to its parent).", [Run, Vector("position", "Position", Xyz(0f, 0f, 0f))]),
            ActionNode("moveBy", "Move By", "Moves the object by an offset.", [Run, Vector("offset", "Offset", Xyz(0f, 0f, 0f))]),
            ActionNode("setRotation", "Set Rotation", "Sets the rotation, in degrees about X, Y and Z.", [Run, Vector("rotation", "Rotation", Xyz(0f, 0f, 0f))]),
            ActionNode("rotateBy", "Rotate By", "Turns the object by degrees about X, Y and Z.", [Run, Vector("degrees", "Degrees", Xyz(0f, 0f, 0f))]),
            ActionNode("setScale", "Set Scale", "Sets the size along X, Y and Z.", [Run, Vector("scale", "Scale", Xyz(1f, 1f, 1f))]),
            Def("branch", "Branch", "Flow", "Continues with True or False, depending on the condition.", [Run, With("condition", "Condition", B, new PinDefault.Bool(false))], [Pin("true", "True", PinType.Flow), Pin("false", "False", PinType.Flow)], [], false),
            Def("number", "Number", "Values", "A fixed number.", [], [Pin("value", "Value", F)], [Setting("value", "Value", new SettingKind.Number(1f))], false),
            Node("vector", "Make Vector", "Values", "A vector from three numbers.", [Number("x", "X", F, 0f), Number("y", "Y", F, 0f), Number("z", "Z", F, 0f)], [Pin("vector", "Vector", V)]),
            Node("split", "Split Vector", "Values", "The three numbers of a vector.", [Pin("vector", "Vector", V)], [Pin("x", "X", F), Pin("y", "Y", F), Pin("z", "Z", F)]),
            GetProperty("property", "Property", "A number property: set per object in the Inspector, else its default in the script's panel.", F),
            GetProperty("wholeProperty", "Whole Number Property", "A whole-number property: set per object in the Inspector, else its default.", PinType.WholeNumber),
            GetProperty("flagProperty", "True/False Property", "A true/false property: set per object in the Inspector, else its default.", B),
            GetProperty("vectorProperty", "Vector Property", "A vector property: set per object in the Inspector, else its default.", V),
            GetProperty("colorProperty", "Color Property", "A colour property: set per object in the Inspector, else its default.", PinType.Color),
            GetProperty("textProperty", "Text Property", "A text property: set per object in the Inspector, else its default.", PinType.Text),
            Def("objectProperty", "Object Property", "Properties", "An object property: the object of the scene chosen per object in the Inspector (by its name); none (Found false) when none is chosen or it is not there.", [], [Pin("value", "Value", PinType.SceneObject), Pin("found", "Found", B)], [PropertyName], false),
            SetProperty("setProperty", "Set Property", "Sets this script's number property on this object, for the rest of play.", Number("value", "Value", F, 0f)),
            SetProperty("setWholeProperty", "Set Whole Number Property", "Sets this script's whole-number property on this object, for the rest of play.", With("value", "Value", PinType.WholeNumber, new PinDefault.WholeNumber(0))),
            SetProperty("setFlagProperty", "Set True/False Property", "Sets this script's true/false property on this object, for the rest of play.", With("value", "Value", B, new PinDefault.Bool(false))),
            SetProperty("setVectorProperty", "Set Vector Property", "Sets this script's vector property on this object, for the rest of play.", Vector("value", "Value", Xyz(0f, 0f, 0f))),
            SetProperty("setColorProperty", "Set Color Property", "Sets this script's colour property on this object, for the rest of play.", With("value", "Value", PinType.Color, new PinDefault.Color(Xyz(1f, 1f, 1f)))),
            SetProperty("setTextProperty", "Set Text Property", "Sets this script's text property on this object, for the rest of play.", TextPin("value", "Value", "")),
            SetProperty("setObjectProperty", "Set Object Property", "Sets this script's object property on this object to the linked object (by its name), for the rest of play.", Pin("value", "Object", PinType.SceneObject)),
            Def("functionStart", "Function", "Functions", "Where a function's actions begin; its outputs are the values the function is called with.", [], [Then], FunctionSettings, false),
            Def("functionReturn", "Return", "Functions", "Ends the function and gives back its outputs to the Call Function that ran it.", [Run], [], FunctionSettings, false),
            Def("callFunction", "Call Function", "Functions", "Runs a function of this script with its inputs, then goes on; its outputs are what the function gave back.", [Run], [Then], FunctionSettings, false),
            Def("eventStart", "On Event", "Events", "Runs when the event is triggered on this object (by a script's Trigger Event, by the interface, or by every object's); its outputs are the values the event brings.", [], [Then], EventSettings, false),
            Def("triggerEvent", "Trigger Event", "Actions", "Triggers an event with its values on the linked object, else the first object named in Object Name, else every object; their On Event nodes run after this step.", [Run, Target], [Then], [EventSettings[0], EventSettings[1], ObjectName], false),
            Node("thisComponent", "This Component", "Find", "This script, as a component of its object: what other scripts trigger events on and subscribe to.", [], [Pin("component", "Component", PinType.Component)]),
            Node("componentObject", "Object of Component", "Find", "The object a component is on, or none.", [ComponentIn], [Pin("object", "Object", PinType.SceneObject)]),
            Def("triggerComponentEvent", "Trigger Component Event", "Actions", "Triggers an event its script declares on one component (a script on an object), with its values; the script's On Event, and the scripts subscribed to it, run after this step. The event is picked from the script's own list.", [Run, ComponentIn], [Then], ComponentEventSettings, false),
            Def("subscribeEvent", "Subscribe to Event", "Actions", "From now on, this script hears the event whenever it happens on the component, with On Component Event; once per component and event.", [Run, ComponentIn], [Then], ComponentEventSettings, false),
            Def("unsubscribeEvent", "Unsubscribe from Event", "Actions", "Stops this script hearing the event of the component it subscribed to.", [Run, ComponentIn], [Then], ComponentEventSettings, false),
            Def("componentEventStart", "On Component Event", "Events", "Runs when the event happens on a component this script subscribed to; Source is that component, and the other outputs are the values the event brings.", [], [Then], ComponentEventSettings, false),
            ListNode("getList", "Get List", "The list itself, to pass on: into a list node, a function or an event.", [], [ListPin]),
            ListNode("listAdd", "Add to List", "Puts the value at the end of the list: the one linked in, else the one named.", [Run, ListPin, ValueIn], [Then]),
            ListNode("listInsert", "Insert into List", "Puts the value in the list at the index (from 0), moving the items from there on along; an index past the end does nothing.", [Run, ListPin, IndexPin, ValueIn], [Then]),
            ListNode("listSet", "Set List Item", "Replaces the list's item at the index (from 0); an index past the end does nothing.", [Run, ListPin, IndexPin, ValueIn], [Then]),
            ListNode("listRemove", "Remove from List", "Takes the item at the index (from 0) out of the list; the items after it move back.", [Run, ListPin, IndexPin], [Then]),
            ListNode("listClear", "Clear List", "Takes every item out of the list.", [Run, ListPin], [Then]),
            ListNode("listItem", "List Item", "The list's item at the index (from 0). Past the end, Found is false and the value is empty.", [ListPin, IndexPin], [ValueOut, Pin("found", "Found", B)]),
            ListNode("listLength", "List Length", "How many items the list holds.", [ListPin], [Pin("length", "Length", PinType.WholeNumber)]),
            ListNode("listFind", "Find in List", "The index of the first item equal to the value, or -1 when there is none.", [ListPin, ValueIn], [Pin("index", "Index", PinType.WholeNumber)]),
            ListNode("listForEach", "For Each in List", "Runs Each once for every item of the list, in order, with the item and its index (from 0); then runs Then. Items added or taken out on the way count.", [Run, ListPin], [Pin("each", "Each", PinType.Flow), ValueOut, Pin("index", "Index", PinType.WholeNumber), Then]),
            MapNode("getMap", "Get Map", "The map itself, to pass on: into a map node, a function or an event.", [], [MapPin]),
            MapNode("mapSet", "Set Map Value", "Keeps the value under the key, in place of what was there, in the map linked in, else the one named.", [Run, MapPin, KeyPin, ValueIn], [Then]),
            MapNode("mapRemove", "Remove from Map", "Takes the key, and its value, out of the map.", [Run, MapPin, KeyPin], [Then]),
            MapNode("mapClear", "Clear Map", "Takes every key out of the map.", [Run, MapPin], [Then]),
            MapNode("mapGet", "Map Value", "The value under the key. When the map has no such key, Found is false and the value is empty.", [MapPin, KeyPin], [ValueOut, Pin("found", "Found", B)]),
            MapNode("mapCount", "Map Count", "How many keys the map holds.", [MapPin], [Pin("count", "Count", PinType.WholeNumber)]),
            MapNode("mapForEach", "For Each in Map", "Runs Each once for every key of the map, in the keys' order (sorted by their characters), with the key and its value; then runs Then. Keys added or taken out on the way count.", [Run, MapPin], [Pin("each", "Each", PinType.Flow), Pin("key", "Key", PinType.Text), ValueOut, Then]),
            Node("position", "Position", "Object", "The object's position, relative to its parent.", [], [Pin("position", "Position", V)]),
            Node("rotation", "Rotation", "Object", "The object's rotation in degrees.", [], [Pin("rotation", "Rotation", V)]),
            Node("scale", "Scale", "Object", "The object's size along X, Y and Z.", [], [Pin("scale", "Scale", V)]),
            Node("time", "Time", "Inputs", "Seconds since play mode started.", [], [Pin("seconds", "Seconds", F)]),
            Node("pointer", "Pointer Position", "Inputs", "Where the pointer is over the game view, from -1 to 1 (x right, y up), whether it is over the view, and whether it is over an interface element that takes it (its clicks are then the interface's, not the game's).", [], [Pin("position", "Position", V), Pin("inside", "Over the View", B), Pin("overInterface", "Over the Interface", B)]),
            Def("mouseDown", "Mouse Button Held", "Inputs", "True while the mouse button (Left, Right or Middle) is held.", [], [Pin("down", "Held", B)], [Setting("button", "Button", new SettingKind.Name("Left"))], false),
            Def("positionOf", "Position Of", "Object", "The position of the linked object, else the first object named in Object Name (relative to its parent).", [Target], [Pin("position", "Position", V)], [ObjectName], false),
            Def("setPositionOf", "Set Position Of", "Actions", "Moves the linked object, else the first object named in Object Name, to a position.", [Run, Target, Vector("position", "Position", Xyz(0f, 0f, 0f))], [Then], [ObjectName], false),
            Def("spawn", "Spawn Prefab", "Actions", "Places a copy of a prefab file at a world position, after this step.", [Run, Vector("position", "Position", Xyz(0f, 0f, 0f))], [Then], [Setting("prefab", "Prefab File", new SettingKind.Text(""))], false),
            Node("thisObject", "This Object", "Find", "The object this script runs on.", [], [Pin("object", "Object", PinType.SceneObject)]),
            Def("dataFile", "Data File", "Values", DataText, [DataFile], [], [DataShape, FieldsSetting], false),
            Def("findNamed", "Find Object Named", "Find", "The first object with this name, in hierarchy order, as the step began. Found is false when there is none.", [TextPin("named", "Name", "")], [Pin("object", "Object", PinType.SceneObject), Pin("found", "Found", B)], [Setting("name", "Name", new SettingKind.Text("Player"))], false),
            Def("findWith", "Find Object With", "Find", "An object that has a component (Point Light, Character…), a tag, or runs a script (by its name), in hierarchy order as the step began: Index 0 is the first. Found is false when there is none.", [Number("index", "Index", F, 0f)], [Pin("object", "Object", PinType.SceneObject), Pin("found", "Found", B)], [SearchKind], false),
            Def("countWith", "Count Objects With", "Find", "How many objects have a component (Point Light, Character…), a tag, or run a script (by its name), as the step began.", [], [Pin("count", "Count", PinType.WholeNumber)], [SearchKind], false),
            Def("forEachWith", "For Each Object With", "Find", "Runs Each once for every object that has a component, a tag, or runs a script, in hierarchy order as the step began, with that object and its index (from 0); then runs Then.", [Run], [Pin("each", "Each", PinType.Flow), Pin("object", "Object", PinType.SceneObject), Pin("index", "Index", PinType.WholeNumber), Then], [SearchKind], false),
            Def("destroyObject", "Destroy Object", "Actions", "Deletes the linked object (else this object) and its children after this step.", [Run, Target], [Then], [], false),
            Def("getComponent", "Get Component", "Find", GetText, [Target], [], [ComponentSetting, FieldsSetting], false),
            Def("tryGetComponent", "Try Get Component", "Find", TryGetText, [Target], [Pin("found", "Found", B)], [ComponentSetting, FieldsSetting], false),
            Def("setComponent", "Set Component", "Actions", SetText, [Run, Target], [Then], [ComponentSetting, FieldsSetting], false),
            ActionNode("playSound", "Play Sound", "Plays the Audio Source of the linked object (else this object), after this step.", [Run, Target]),
            ActionNode("stopSound", "Stop Sound", "Stops the sounds the linked object (else this object) plays, after this step.", [Run, Target]),
            ActionNode("playKeyframes", "Play Keyframe Animation", "Plays the Keyframe Animation of the linked object (else this object) from its start, after this step.", [Run, Target]),
            ActionNode("stopKeyframes", "Stop Keyframe Animation", "Stops the Keyframe Animation of the linked object (else this object) where it is, after this step.", [Run, Target]),
            Def("playSoundAt", "Play Sound At", "Actions", "Plays a sound file once from a world position, after this step, heard from the first camera.", [Run, Vector("position", "Position", Xyz(0f, 0f, 0f)), Number("volume", "Volume", F, 1f)], [Then], [Setting("sound", "Sound File", new SettingKind.Text(""))], false),
            Def("loadScene", "Load Scene", "Actions", "After this step, loads a scene file: in place of the playing scene (every script starts again; the game's values stay), or, with Add to This Scene, its objects join this one and Unload Scene removes them again.", [Run, With("additive", "Add to This Scene", B, new PinDefault.Bool(false))], [Then], [SceneFile], false),
            Def("unloadScene", "Unload Scene", "Actions", "After this step, removes the objects of the scenes added from this file while playing.", [Run], [Then], [SceneFile], false),
            ActionNode("destroySelf", "Destroy This Object", "Deletes this object and its children after this step.", [Run]),
            Def("gameValue", "Game Value", "Game", "A number the game keeps under a name: the game's own (Scope Game), a player's (Scope Player: the object named in Player, else the first player) or this object's (Scope This Object, when it has Player Control); Scope Saved keeps it from one play to the next, as for a best score. Nothing is set at first: Value is 0 and Is Set is false until something sets it.", [], [Pin("value", "Value", F), Pin("set", "Is Set", B)], [StateName, StateScope, StatePlayer], false),
            Def("setGameValue", "Set Game Value", "Game", "Keeps a number under a name in the game's state or a player's (see Game Value for the scopes).", [Run, Number("value", "Value", F, 0f)], [Then], [StateName, StateScope, StatePlayer], false),
            Def("changeGameValue", "Change Game Value", "Game", "Adds an amount to a number in the game's state or a player's; a name not set yet counts as 0.", [Run, Number("amount", "Amount", F, 1f)], [Then], [StateName, StateScope, StatePlayer], false),
            Def("setInputMap", "Set Input Map", "Inputs", "Makes another map of an Input Map active (Player, Menu…): the project's, or this object's own when it has an Input Map component. Only the active map's actions work.", [Run], [Then], [Setting("map", "Map", new SettingKind.Name("Menu")), Setting("scope", "Scope", new SettingKind.Choice(["Project", "This Object"]))], false),
            Def("endGame", "End Game", "Game", "Ends the game, won or lost; every script then gets the message gameWon or gameLost. Only the first ending counts.", [Run, With("won", "Won", B, new PinDefault.Bool(true))], [Then], [], false),
            Def("playParticles", "Play Particles", "Effects", "Starts the Particle Emitter of the linked object, else the first object named in Object Name, else this object's: a new cycle, with its burst. After this step.", [Run, Target], [Then], [ObjectName], false),
            Def("stopParticles", "Stop Particles", "Effects", "Stops the emitter starting particles; those alive live out their lives. Which emitter: as Play Particles.", [Run, Target], [Then], [ObjectName], false),
            Def("burstParticles", "Burst Particles", "Effects", "Starts Count particles at once from the emitter (as many as it has room for). Which emitter: as Play Particles.", [Run, Target, With("count", "Count", PinType.WholeNumber, new PinDefault.WholeNumber(20))], [Then], [ObjectName], false),
            Def("playClip", "Play Clip", "Animation", "Plays the clip named in Clip on the model of the linked object, else the first object named in Object Name, else this object, fading from what played over Fade seconds (0 to 60). After this step.", [Run, Target, With("clip", "Clip", PinType.Text, new PinDefault.Text("")), With("fade", "Fade", PinType.Number, new PinDefault.Number(0.25f)), With("loop", "Loop", B, new PinDefault.Bool(true))], [Then], [ObjectName], false),
            Def("stopClip", "Stop Clip", "Animation", "Fades the model to its rest pose over Fade seconds. Which object: as Play Clip.", [Run, Target, With("fade", "Fade", PinType.Number, new PinDefault.Number(0.25f))], [Then], [ObjectName], false),
            Def("setAnimationNumber", "Set Animation Number", "Animation", "Sets the number parameter named in Parameter, which mixes and animation states read. Which object: as Play Clip.", [Run, Target, With("value", "Value", PinType.Number, new PinDefault.Number(0f))], [Then], [AnimationParameter, ObjectName], false),
            Def("setAnimationFlag", "Set Animation Flag", "Animation", "Sets the true/false parameter named in Parameter. Which object: as Play Clip.", [Run, Target, With("value", "Value", B, new PinDefault.Bool(false))], [Then], [AnimationParameter, ObjectName], false),
            Def("triggerAnimation", "Trigger Animation", "Animation", "Sets the trigger named in Parameter, which the first transition it lets through uses up. Which object: as Play Clip.", [Run, Target], [Then], [AnimationParameter, ObjectName], false),
            Def("animationState", "Animation State", "Animation", "The animation state the object is in, as of the last step (empty, and In States false, when it runs none). Which object: as Play Clip.", [Target], [Pin("state", "State", PinType.Text), Pin("running", "In States", B)], [ObjectName], false),
            Def("jointPosition", "Joint Position", "Animation", "Where the joint named in Joint of the object's model is in the world, as the last step posed it (Found is false when the object is not animated or has no such joint). Which object: as Play Clip.", [Target], [Pin("position", "Position", V), Pin("found", "Found", B)], [Setting("joint", "Joint", new SettingKind.Text("")), ObjectName], false),
            Def("interfacePressed", "On Interface Pressed", "Interface", "Runs when the player presses the button named in Element, in this object's Interface.", [], [Then], [ElementName], false),
            Def("interfaceChanged", "On Interface Changed", "Interface", "Runs when the player changes the toggle, slider or text field named in Element, in this object's Interface. Value is its new value (a text field's length).", [], [Then, Pin("value", "Value", F)], [ElementName], false),
            Def("setInterfaceText", "Set Interface Text", "Interface", "Sets the text of the element named in Element, in the Interface of the linked object, else the first object named in Object Name, else this object. It shows after this step.", [Run, Target, TextPin("text", "Text", "")], [Then], [ElementName, ObjectName], false),
            Def("setInterfaceValue", "Set Interface Value", "Interface", "Sets an element's value: a toggle's state (0 or 1), a slider's number, a progress bar's fill (0 to 1). Which interface: as Set Interface Text.", [Run, Target, Number("value", "Value", F, 0f)], [Then], [ElementName, ObjectName], false),
            Def("showInterface", "Show Interface", "Interface", "Shows or hides the element named in Element, or the whole interface when Element is empty. Which interface: as Set Interface Text.", [Run, Target, With("visible", "Visible", B, new PinDefault.Bool(true))], [Then], [Setting("element", "Element", new SettingKind.Text("")), ObjectName], false),
            Def("addInterfaceElement", "Add Interface Element", "Interface", "Builds the interface while the game plays: adds an element named Name into the element named in Into (the root when empty). What is a kind (button, text, progress...) or a layout file (.bui) used as a part, whose elements are then named Name/element.", [Run, Target, TextPin("name", "Name", "Item")], [Then], [Setting("into", "Into", new SettingKind.Text("")), Setting("what", "What", new SettingKind.Text("button")), ObjectName], false),
            Def("interfaceValue", "Interface Value", "Interface", "The value of the element named in Element as it shows now: a toggle's 0 or 1, a slider's number, a bound game value. Found is false when the interface has no such element. Which interface: the linked object's, else the first object named in Object Name, else this object's.", [Target], [Pin("value", "Value", F), Pin("found", "Found", B)], [ElementName, ObjectName], false),
            Def("interfaceText", "Interface Text", "Interface", "The text of the element named in Element as it shows now, what the player typed into a text field too; empty when there is no such element. Which interface: as Interface Value.", [Target], [Pin("text", "Text", PinType.Text)], [ElementName, ObjectName], false),
            Def("removeInterfaceElement", "Remove Interface Element", "Interface", "Removes the element named Name, with what it holds. Which interface: as Set Interface Text.", [Run, Target, TextPin("name", "Name", "Item")], [Then], [ObjectName], false),
            Def("walk", "Walk", "Physics", "Sets a character's walking velocity (the linked object, else the first one named in Object Name, else this object), in world units per second, until changed.", [Run, Target, Vector("velocity", "Velocity", Xyz(0f, 0f, 0f))], [Then], [ObjectName], false),
            Def("jump", "Jump", "Physics", "Makes a character on the ground jump with an upward speed (the linked object, else the first one named in Object Name, else this object).", [Run, Target, Number("speed", "Speed", F, 5f)], [Then], [ObjectName], false),
            Node("onGround", "On Ground", "Physics", "True while this object's character stands on something.", [], [Pin("grounded", "On Ground", B)]),
            Def("addImpulse", "Add Impulse", "Physics", "Pushes a physics body (the linked object, else the first one named in Object Name, else this object) by an impulse in world space, after this step.", [Run, Target, Vector("impulse", "Impulse", Xyz(0f, 5f, 0f))], [Then], [ObjectName], false),
            Def("setVelocity", "Set Velocity", "Physics", "Sets a physics body's velocity (the linked object, else the first one named in Object Name, else this object), in world units per second, after this step.", [Run, Target, Vector("velocity", "Velocity", Xyz(0f, 0f, 0f))], [Then], [ObjectName], false),
            Node("velocity", "Velocity", "Physics", "This object's physics velocity as the step began (0 without a body).", [], [Pin("velocity", "Velocity", V)]),
            Def("raycast", "Cast Ray", "Object", "Sends a ray from Origin along Direction, up to Distance, and says whether it hits another object's shape or sprite, how far, and where.", [Vector("origin", "Origin", Xyz(0f, 0f, 0f)), Vector("direction", "Direction", Xyz(0f, -1f, 0f)), Number("distance", "Distance", F, 100f)], [Pin("hit", "Hit", B), Pin("distance", "Hit Distance", F), Pin("point", "Hit Point", V)], [], false),
            Def("padStick", "Gamepad Stick", "Inputs", "Where a gamepad stick (Left or Right) points, from -1 to 1 (x right, y up); 0 without a gamepad.", [], [Pin("direction", "Direction", V), Pin("connected", "Gamepad Connected", B)], [Setting("stick", "Stick", new SettingKind.Name("Left"))], false),
            Def("padTrigger", "Gamepad Trigger", "Inputs", "How far a gamepad trigger (Left or Right) is pressed, from 0 to 1.", [], [Pin("amount", "Amount", F)], [Setting("trigger", "Trigger", new SettingKind.Name("Right"))], false),
            Def("padButton", "Gamepad Button Held", "Inputs", "True while the gamepad button is held: South, East, West, North, LeftBumper, RightBumper, Select, Start, Up, Down, Left, Right and others.", [], [Pin("down", "Held", B)], [Setting("button", "Button", new SettingKind.Name("South"))], false),
            Def("keyDown", "Key Held", "Inputs", "True while the key is held (names like W, Space, ArrowUp).", [], [Pin("down", "Held", B)], [Setting("key", "Key", new SettingKind.Name("Space"))], false),
            Node("add", "Add", "Math", "A + B.", [Number("a", "A", A, 0f), Number("b", "B", A, 0f)], MathOut),
            Node("subtract", "Subtract", "Math", "A − B.", [Number("a", "A", A, 0f), Number("b", "B", A, 0f)], MathOut),
            Node("multiply", "Multiply", "Math", "A × B.", [Number("a", "A", A, 1f), Number("b", "B", A, 1f)], MathOut),
            Node("negate", "Negate", "Math", "The value with its sign turned: 5 becomes −5 and −5 becomes 5; a vector points the other way.", [Number("value", "Value", A, 0f)], MathOut),
            Node("divide", "Divide", "Math", "A ÷ B.", [Number("a", "A", A, 1f), Number("b", "B", A, 1f)], MathOut),
            Node("sine", "Sine", "Math", "The sine of the input, in radians.", [Number("value", "Value", A, 0f)], MathOut),
            Node("power", "Power", "Math", "Base to the power of the exponent: whole numbers stay whole (a negative exponent gives 0, or ±1 for 1 and −1); for a vector, each part.", [Number("base", "Base", A, 1f), Number("exponent", "Exponent", F, 2f)], MathOut),
            Node("squareRoot", "Square Root", "Math", "The number that times itself gives the input (0 below 0); for a vector, of each part.", [Number("value", "Value", A, 1f)], MathOut),
            Node("vectorLength", "Vector Length", "Math", "How long a vector is: the distance from 0 to the point it names.", [Vector("vector", "Vector", Xyz(0f, 0f, 0f))], [Pin("length", "Length", F)]),
            Node("normalize", "Normalize", "Math", "The vector's direction at length 1: a move as fast diagonally as straight. A vector of length 0 stays 0, so a player standing still stays still.", [Vector("vector", "Vector", Xyz(0f, 0f, 0f))], [Pin("vector", "Vector", V)]),
            Node("lerp", "Lerp", "Math", "Linear interpolation: from A (factor 0) to B (factor 1) on a straight line; factors outside 0 to 1 go past A or B. For vectors, each part.", [Number("a", "A", A, 0f), Number("b", "B", A, 1f), Number("factor", "Factor", F, 0.5f)], MathOut),
            Node("inverseLerp", "Inverse Lerp", "Math", "Where the value lies from A to B: 0 at A, 1 at B, 0.5 halfway, and past 0 or 1 beyond them; 0 when A and B are equal.", [Number("a", "A", F, 0f), Number("b", "B", F, 1f), Number("value", "Value", F, 0.5f)], [Pin("result", "Result", F)]),
            Node("choose", "Choose", "Math", "A when the condition is true, otherwise B.", [With("condition", "Condition", B, new PinDefault.Bool(false)), Number("a", "A", A, 1f), Number("b", "B", A, 0f)], MathOut),
            Node("greater", "Greater Than", "Logic", "True when A > B.", [Number("a", "A", F, 0f), Number("b", "B", F, 0f)], [Pin("result", "Result", B)]),
            Node("and", "And", "Logic", "True when both are true.", [Pin("a", "A", B), Pin("b", "B", B)], [Pin("result", "Result", B)]),
            Node("or", "Or", "Logic", "True when either is true.", [Pin("a", "A", B), Pin("b", "B", B)], [Pin("result", "Result", B)]),
            Node("not", "Not", "Logic", "True when the input is false.", [Pin("value", "Value", B)], [Pin("result", "Result", B)]),
            Node("wholeNumber", "Whole Number", "Values", "A whole number typed here.", [With("value", "Value", PinType.WholeNumber, new PinDefault.WholeNumber(0))], [Pin("result", "Result", PinType.WholeNumber)]),
            Def("toWhole", "To Whole Number", "Math", "A number made whole: Nearest (halves away from zero), Down, Up or Toward Zero. Numbers past the 32-bit range stop at its ends.", [Number("value", "Value", F, 0f)], [Pin("result", "Result", PinType.WholeNumber)], [Setting("rounding", "Rounding", new SettingKind.Choice(["Nearest", "Down", "Up", "Toward Zero"]))], false),
            Node("remainder", "Remainder", "Math", "What is left after dividing A by B as whole numbers (its sign is A's); 0 when B is 0.", [With("a", "A", PinType.WholeNumber, new PinDefault.WholeNumber(0)), With("b", "B", PinType.WholeNumber, new PinDefault.WholeNumber(1))], [Pin("result", "Result", PinType.WholeNumber)]),
            Node("text", "Text", "Text", "Text typed here.", [TextPin("value", "Value", "")], TextOut),
            Node("joinText", "Join Text", "Text", "A followed by B.", [TextPin("a", "A", ""), TextPin("b", "B", "")], TextOut),
            Node("textLength", "Text Length", "Text", "How many characters the text has.", [TextPin("text", "Text", "")], [Pin("result", "Length", PinType.WholeNumber)]),
            Node("numberToText", "To Text", "Text", "A number (up to three decimals), a whole number (every digit) or a vector, as (x, y, z), written as text.", [Number("value", "Value", A, 0f)], TextOut),
            Node("textToNumber", "Text to Number", "Text", "The number the text spells, such as -2.5 (spaces around it are fine). Valid is false, and the number 0, when it spells none.", [TextPin("text", "Text", "")], [Pin("result", "Number", F), Pin("valid", "Valid", B)]),
            Node("sameText", "Same Text", "Text", "True when A and B are the same text (upper and lower case differ).", [TextPin("a", "A", ""), TextPin("b", "B", "")], [Pin("result", "Result", B)]),
            Node("textContains", "Text Contains", "Text", "True when Part appears in the text.", [TextPin("text", "Text", ""), TextPin("part", "Part", "")], [Pin("result", "Result", B)]),
            Node("startsWith", "Starts With", "Text", "True when the text begins with Part.", [TextPin("text", "Text", ""), TextPin("part", "Part", "")], [Pin("result", "Result", B)]),
            Node("endsWith", "Ends With", "Text", "True when the text ends with Part.", [TextPin("text", "Text", ""), TextPin("part", "Part", "")], [Pin("result", "Result", B)]),
            Node("findText", "Find in Text", "Text", "Where Part first appears in the text, counted in characters from 0; -1 when it does not.", [TextPin("text", "Text", ""), TextPin("part", "Part", "")], [Pin("result", "Index", PinType.WholeNumber)]),
            Node("textPart", "Part of Text", "Text", "Count characters of the text from Start (0 is the first), cut short at its end.", [TextPin("text", "Text", ""), Number("start", "Start", F, 0f), Number("count", "Count", F, 1f)], TextOut),
            Node("replaceText", "Replace Text", "Text", "The text with every Find replaced by With.", [TextPin("text", "Text", ""), TextPin("find", "Find", ""), TextPin("with", "With", "")], TextOut),
            Node("upperCase", "Upper Case", "Text", "The text with a to z made A to Z.", [TextPin("text", "Text", "")], TextOut),
            Node("lowerCase", "Lower Case", "Text", "The text with A to Z made a to z.", [TextPin("text", "Text", "")], TextOut),
            Node("trimText", "Trim Text", "Text", "The text without spaces, tabs and line breaks at its start and end.", [TextPin("text", "Text", "")], TextOut),
            Node("nameOf", "Name Of", "Text", "The name of the linked object, else this object.", [Target], TextOut),
        ],
    };
}

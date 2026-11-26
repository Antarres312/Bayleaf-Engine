using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

namespace Bayleaf.Scene.Tests;

public class SceneTests
{
    private static JsonValue Json(string text) => JsonReader.Parse(text).Value;

    /// <summary>root A { Child { Grandchild } }, root B.</summary>
    private static (Scene Scene, StableId A, StableId Child, StableId Grandchild, StableId B) Tree()
    {
        var scene = new Scene();
        var a = scene.CreateObject(null, "A").Value;
        var child = scene.CreateObject(a, "Child").Value;
        var grandchild = scene.CreateObject(child, "Grandchild").Value;
        var b = scene.CreateObject(null, "B").Value;
        return (scene, a, child, grandchild, b);
    }

    [Fact]
    public void NewObjectsHaveTheRequiredComponents()
    {
        var (scene, a, _, _, _) = Tree();
        var names = scene.Components(a).Value.Select(c => c.Name).ToList();
        Assert.Contains("name", names);
        Assert.Contains("transform", names);
        Assert.Equal("A", scene.NameOf(a));
    }

    [Fact]
    public void DeletingAnObjectDeletesItsDescendantsAndTheirEntities()
    {
        var (scene, a, child, grandchild, b) = Tree();
        Assert.Equal(3, scene.DeleteObject(a).Value);
        Assert.False(scene.Contains(child) || scene.Contains(grandchild));
        Assert.Equal([b], scene.Children(null));
        Assert.Equal(1, scene.World.Count);
    }

    [Fact]
    public void ReparentingUnderADescendantIsRefusedAndChangesNothing()
    {
        var (scene, a, _, grandchild, _) = Tree();
        var before = scene.ToDoc();
        Assert.Equal(SceneErrorKind.Cycle, scene.Reparent(a, grandchild, null).Error.Kind);
        Assert.Equal(SceneErrorKind.Cycle, scene.Reparent(a, a, null).Error.Kind);
        Assert.Equal(before, scene.ToDoc());
    }

    [Fact]
    public void ReparentPlacesTheObjectAtTheRequestedIndex()
    {
        var (scene, a, child, _, b) = Tree();
        Assert.True(scene.Reparent(b, a, 0).IsOk);
        Assert.Equal([b, child], scene.Children(a));
        Assert.Equal(a, scene.Parent(b));
        Assert.True(scene.Reparent(b, null, 99).IsOk);
        Assert.Equal([a, b], scene.Children(null));
    }

    [Fact]
    public void InvalidComponentValuesAreRefused()
    {
        var (scene, a, _, _, _) = Tree();
        var before = scene.Component(a, "transform").Value;
        Assert.True(scene.SetComponent(a, "transform", Json("{\"position\": \"up\"}")).IsError);
        Assert.True(scene.SetComponent(a, "transform", Json("{\"position\": [0, 0, 0], \"rotation\": [0, 0, 0], \"scale\": [1e300, 1, 1]}")).IsError);
        Assert.True(scene.SetComponent(a, "nothing", Json("1")).IsError);
        Assert.Equal(before, scene.Component(a, "transform").Value);
    }

    [Fact]
    public void RequiredComponentsCannotBeRemovedButOptionalOnesCan()
    {
        var (scene, a, _, _, _) = Tree();
        Assert.Equal(SceneErrorKind.NotRemovable, scene.RemoveComponent(a, "transform").Error.Kind);
        Assert.True(scene.AddComponent(a, "tags").IsOk);
        Assert.True(scene.AddComponent(a, "tags").IsError);
        Assert.True(scene.RemoveComponent(a, "tags").IsOk);
        Assert.True(scene.Component(a, "tags").IsError);
    }

    [Fact]
    public void SaveAndLoadRoundTripKeepsHierarchyOrderAndValues()
    {
        var (scene, a, child, _, b) = Tree();
        scene.Reparent(b, a, 0);
        Assert.True(scene.SetComponent(child, "transform", Json("{\"position\": [1.5, 2.0, -3.0], \"rotation\": [0, 90, 0], \"scale\": [2, 2, 2]}")).IsOk);
        scene.AddComponent(child, "tags");
        Assert.True(scene.SetComponent(child, "tags", Json("[\"enemy\"]")).IsOk);
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-scene-test-{System.Environment.ProcessId}");
        Directory.CreateDirectory(dir);
        try
        {
            var path = Path.Join(dir, $"round-trip.{SceneDoc.Extension}");
            Assert.True(scene.Save(path).IsOk);
            Assert.Equal(scene.ToDoc(), Scene.Load(path).Value.ToDoc());
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }

    [Fact]
    public void WorldMatricesComposeParentTransforms()
    {
        var (scene, a, child, grandchild, _) = Tree();
        scene.SetComponent(a, "transform", Json("{\"position\": [10, 0, 0], \"rotation\": [0, 0, 0], \"scale\": [2, 2, 2]}"));
        scene.SetComponent(child, "transform", Json("{\"position\": [1, 0, 0], \"rotation\": [0, 0, 0], \"scale\": [1, 1, 1]}"));
        var all = scene.WorldMatrices();
        var fromList = all.First(m => m.Id == grandchild).Matrix;
        var direct = scene.WorldMatrix(grandchild)!.Value;
        Assert.True(fromList.NearlyEquals(direct, 1e-5));
        // The parent's scale stretches the child's offset: 10 + 2 * 1.
        Assert.True(Math.Abs(direct.WAxis.X - 12) < 1e-5);
        int At(StableId id) => all.ToList().FindIndex(m => m.Id == id);
        Assert.True(At(a) < At(child) && At(child) < At(grandchild), "parents come before children");
    }

    [Fact]
    public void ReparentingCanKeepTheWorldPlacement()
    {
        var (scene, a, _, _, b) = Tree();
        scene.SetComponent(a, "transform", Json("{\"position\": [3, 1, 0], \"rotation\": [0, 90, 0], \"scale\": [2, 2, 2]}"));
        scene.SetComponent(b, "transform", Json("{\"position\": [5, 0.5, -2], \"rotation\": [10, 0, 0], \"scale\": [1, 1, 1]}"));
        var before = scene.WorldMatrix(b)!.Value;
        Assert.True(scene.ReparentKeepingWorld(b, a, null).IsOk);
        Assert.True(scene.WorldMatrix(b)!.Value.NearlyEquals(before, 1e-3));
        Assert.True(scene.ReparentKeepingWorld(b, null, null).IsOk);
        Assert.True(scene.WorldMatrix(b)!.Value.NearlyEquals(before, 1e-3));
    }

    [Fact]
    public void EulerAnglesComeBackFromTheirMatrix()
    {
        foreach (var rotation in new[] { new Vector3(10, -40, 75), new Vector3(-120, 30, 5), new Vector3(0, 89, 0), new Vector3(45, 0, -170) })
        {
            var transform = new Transform(new Double3(1, 2, 3), rotation, new Vector3(1, 2, 0.5f));
            var back = Transform.FromMatrix(transform.Matrix());
            Assert.True(back.Matrix().NearlyEquals(transform.Matrix(), 1e-5), $"{rotation} came back as {back.Rotation}");
        }
    }

    [Fact]
    public void RenderComponentsValidateTheirRanges()
    {
        var (scene, a, _, _, _) = Tree();
        foreach (var (component, bad) in new[]
        {
            ("mesh", "{\"shape\": \"teapot\", \"color\": [1, 1, 1, 1], \"metallic\": 0, \"roughness\": 0.5}"),
            ("mesh", "{\"shape\": \"cube\", \"color\": [2, 1, 1, 1], \"metallic\": 0, \"roughness\": 0.5}"),
            ("camera", "{\"projection\": \"perspective\", \"fov\": 0, \"size\": 5, \"near\": 0.1, \"far\": 100}"),
            ("camera", "{\"projection\": \"perspective\", \"fov\": 60, \"size\": 5, \"near\": 10, \"far\": 1}"),
            ("directionalLight", "{\"color\": [1, 1, 1], \"intensity\": -1}"),
            ("sprite", "{\"color\": [1, 1, 1, 1], \"size\": [0, 1]}"),
        })
        {
            scene.AddComponent(a, component);
            Assert.True(scene.SetComponent(a, component, Json(bad)).IsError, $"{component} accepted {bad}");
        }
    }

    [Fact]
    public void LoadingRejectsBrokenDocuments()
    {
        var (scene, _, _, _, _) = Tree();
        var good = scene.ToDoc();
        var objects = good.Objects.Select(o => o.Copy()).ToList();
        var unknown = objects.Select(o => o.Copy()).ToList();
        unknown[0].Components["mystery"] = Json("1");
        foreach (var (what, broken) in new[]
        {
            ("format", good with { Format = "something.else" }),
            ("order", good with { Objects = [objects[1], objects[0], .. objects.Skip(2)] }),
            ("duplicate", good with { Objects = [.. objects, objects[0]] }),
            ("component", good with { Objects = unknown }),
        })
        {
            Assert.True(Scene.FromDoc(broken).IsError, $"accepted a document with a bad {what}");
        }
    }

    [Fact]
    public void Version1ScenesLoadWithTheirScriptMovedIntoTheList()
    {
        var doc = SceneDoc.FromText("""
            {"format": "bayleaf.scene", "version": 1, "objects": [{"id": "0123456789abcdef0123456789abcdef", "parent": null, "components": {
                "name": "Old", "script": {"module": "scripts/spinner.wasm", "properties": {"speed": 60}}}}]}
            """).Value;
        var scene = Scene.FromDoc(doc).Value;
        var scripts = scene.Get<Scripts>(scene.Children(null)[0]).Value;
        Assert.Single(scripts.Items);
        Assert.Equal("Spinner", scripts.Items[0].DisplayName());
        Assert.Equal(SceneDoc.CurrentVersion, scene.ToDoc().Version);
    }

    [Fact]
    public void ScriptNamesComeFromTheModuleFile()
    {
        static string Name(string module) => new ScriptRef(module).DisplayName();
        Assert.Equal("Player Controller", Name("scripts/player_controller.wasm"));
        Assert.Equal("Mover", Name(@"a\b\mover.wasm"));
        Assert.Equal("Door Opener", Name("scripts/door_opener.bscript"));
    }

    [Fact]
    public void ExportedSubtreesImportAsCopiesWithNewIdsInPlace()
    {
        var scene = new Scene();
        var a = scene.CreateObject(null, "A").Value;
        var b = scene.CreateObject(null, "B").Value;
        var child = scene.CreateObject(a, "Child").Value;
        scene.CreateObject(child, "Grandchild");
        scene.SetComponent(a, "transform", Json("{\"position\": [1, 2, 3], \"rotation\": [0, 0, 0], \"scale\": [1, 1, 1]}"));

        // A root inside another root is copied once.
        var copied = scene.Export([a, child]).Value;
        Assert.Equal(3, copied.Count);
        var tops = scene.Import(copied, null, 1).Value;
        Assert.Single(tops);
        var copy = tops[0];
        Assert.NotEqual(a, copy);
        Assert.Equal([a, copy, b], scene.Children(null));
        Assert.Equal(scene.Component(a, "transform").Value, scene.Component(copy, "transform").Value);
        var copyChild = scene.Children(copy)[0];
        Assert.Equal("Child", scene.NameOf(copyChild));
        Assert.Single(scene.Children(copyChild));
        Assert.True(scene.IsInside(scene.Children(copyChild)[0], copy));

        // Broken data is refused.
        Assert.True(scene.Import(copied.Skip(1).ToList(), null, null).IsError, "a child listed without its parent");
        var bad = copied.Select(o => o.Copy()).ToList();
        bad[0].Components["transform"] = Json("{\"position\": \"up\"}");
        Assert.True(scene.Import(bad, null, null).IsError);
    }

    [Fact]
    public void AnInspectorLayoutIsCheckedAgainstTheDeclaredProperties()
    {
        DeclaredProperty[] declared =
        [
            new("speed", PropertyType.Number, PropertyShape.One, new PropertyValue.Number(1)),
            new("mode", PropertyType.WholeNumber, PropertyShape.One, new PropertyValue.Number(0)),
            new("label", PropertyType.Text, PropertyShape.One, new PropertyValue.Text("")),
        ];
        var layout = Json("""
            [
              {"group": "Movement", "closed": true, "items": [
                {"property": "speed", "label": "Top speed", "hint": "Units a second", "range": [0, 20], "slider": true, "showIf": ["mode", 1]},
                {"button": "Jump", "message": "jump"}
              ]},
              {"property": "mode", "choices": ["Walk", "Run"]},
              {"note": "Careful", "warning": true}
            ]
            """);
        var items = InspectorItem.Parse(layout, declared).Value;
        var group = Assert.IsType<InspectorItem.Group>(items[0]);
        Assert.True(group.Closed);
        var speed = Assert.IsType<InspectorItem.Row>(group.Items[0]).Layout;
        Assert.Equal((0.0, 20.0), speed.Range);
        Assert.True(speed.Slider);
        Assert.True(speed.ShowIf!.Holds(new PropertyValue.Number(1)));
        Assert.False(speed.ShowIf.Holds(new PropertyValue.Number(0)));

        foreach (var (bad, why) in new[]
        {
            ("[{\"property\": \"nope\"}]", "does not declare"),
            ("[{\"property\": \"speed\"}, {\"property\": \"speed\"}]", "twice"),
            ("[{\"property\": \"label\", \"range\": [0, 1]}]", "a range is for numbers"),
            ("[{\"property\": \"speed\", \"range\": [5, 1]}]", "lower first"),
            ("[{\"property\": \"speed\", \"slider\": true}]", "needs a range"),
            ("[{\"property\": \"speed\", \"choices\": [\"a\"]}]", "choices are for"),
            ("[{\"button\": \"Go\"}]", "needs the message"),
            ("[{\"note\": \"x\", \"group\": \"y\"}]", "exactly one"),
            ("[{\"note\": \"x\", \"slider\": true}]", "does not go with"),
            ("[{\"property\": \"speed\", \"showIf\": [\"mode\", \"fast\"]}]", "is not a"),
            ("[{\"group\": \"a\", \"items\": [{\"group\": \"b\", \"items\": [{\"group\": \"c\", \"items\": [{\"group\": \"d\", \"items\": [{\"group\": \"e\"}]}]}]}]}]", "nest"),
        })
        {
            var error = InspectorItem.Parse(Json(bad), declared);
            Assert.True(error.IsError, why);
            Assert.Contains(why, error.Error, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void PostProcessingAntialiasingReadsOldOnAndOffAndTheNewChoices()
    {
        static string File(int version, string value) => "{\"format\": \"bayleaf.post\", \"version\": " + version + ", \"post\": {\"antialiasing\": " + value + "}}";
        Assert.Equal(Antialiasing.Fast, PostProcessing.FromText(File(2, "true")).Value.Antialiasing);
        Assert.Equal(Antialiasing.Off, PostProcessing.FromText(File(2, "false")).Value.Antialiasing);
        Assert.Equal(Antialiasing.Fine, PostProcessing.FromText(File(3, "\"fine\"")).Value.Antialiasing);
        Assert.True(PostProcessing.FromText(File(3, "\"best\"")).IsError, "only off, fast or fine");
        Assert.Contains("\"antialiasing\": \"fine\"", (PostProcessing.Default with { Antialiasing = Antialiasing.Fine }).ToText(), StringComparison.Ordinal);
    }

    [Fact]
    public void ListAndMapScriptPropertiesComeBackFromTheFileAsTheyWent()
    {
        var scene = new Scene();
        var id = scene.CreateObject(null, "Spawner").Value;
        // Three numbers in a list are a list, not a vector.
        var list = new PropertyValue.List([new PropertyValue.Number(1), new PropertyValue.Number(2), new PropertyValue.Number(3)]);
        var map = new PropertyValue.Map(new SortedDictionary<string, PropertyValue>(Utf8Order.Instance) { ["easy"] = new PropertyValue.Number(3) });
        var script = new ScriptRef("waves.bscript", new SortedDictionary<string, PropertyValue>(Utf8Order.Instance) { ["waves"] = list, ["by"] = map });
        Assert.True(scene.Set(id, new Scripts([script])).IsOk);
        var back = Scene.FromDoc(SceneDoc.FromText(scene.ToDoc().ToJson().ToCompact()).Value).Value;
        var properties = back.Get<Scripts>(id).Value.Items[0].Properties;
        Assert.Equal(list, properties["waves"]);
        Assert.Equal(map, properties["by"]);
        Assert.True(list.FitsShape(PropertyType.WholeNumber, PropertyShape.List));
        Assert.False(list.FitsShape(PropertyType.Vector, PropertyShape.One));
    }

    [Fact]
    public void AKeySetsItsObjectsTransformUntilTheObjectIsDeleted()
    {
        var (scene, a, _, _, b) = Tree();
        var key = scene.Key(a)!.Value;
        var mark = scene.TransformMark;
        var moved = Transform.Initial with { Position = new Double3(3, 0, 0) };
        Assert.True(scene.SetTransform(key, moved));
        Assert.Equal(moved, scene.Get<Transform>(a).Value);
        Assert.Equal([a], scene.TransformsWrittenSince(mark)!);
        Assert.False(scene.SetTransform(key, Transform.Initial with { Position = new Double3(double.NaN, 0, 0) }), "not finite");
        scene.DeleteObject(a);
        // The deleted object's place may be another object's now; the key reaches neither.
        var c = scene.CreateObject(null, "C").Value;
        Assert.False(scene.SetTransform(key, moved));
        Assert.NotEqual(moved, scene.Get<Transform>(c).Value);
        Assert.NotEqual(moved, scene.Get<Transform>(b).Value);
    }

    [Fact]
    public void TransformWritesSinceAMarkAreListedAndAMarkTooOldSaysSo()
    {
        var (scene, a, child, _, b) = Tree();
        var mark = scene.TransformMark;
        var moved = new Transform(new Double3(1, 2, 3), Vector3.Zero, Vector3.One);
        scene.Set(a, moved);
        scene.SetComponent(b, "transform", Json("{\"position\": [4, 5, 6], \"rotation\": [0, 0, 0], \"scale\": [1, 1, 1]}"));
        scene.Set(child, new Name("Renamed"));
        Assert.Equal([a, b], scene.TransformsWrittenSince(mark)!);
        Assert.Empty(scene.TransformsWrittenSince(scene.TransformMark)!);
        // Many more writes than objects: the log starts again, and the first mark no longer reaches.
        for (var i = 0; i < 5000; i++)
        {
            scene.Set(a, moved);
        }
        Assert.Null(scene.TransformsWrittenSince(mark));
    }

    [Fact]
    public void DataFilesReadTheirValuesWhereTheyFitAndTheShapeDefaultsElsewhere()
    {
        var health = DataField.Of("health", PropertyType.Number, PropertyShape.One) with { Default = new PropertyValue.Number(100) };
        var waves = DataField.Of("waves", PropertyType.WholeNumber, PropertyShape.List) with { Default = new PropertyValue.List([new PropertyValue.Number(3)]) };
        var shape = new DataShape([health, waves, DataField.Of("drop", PropertyType.Prefab, PropertyShape.One)]);
        Assert.True(shape.Validate().IsOk);
        var data = new DataFile("data/enemy.bshape");
        data.Values["health"] = new PropertyValue.Number(40);
        data.Values["drop"] = new PropertyValue.Text("coin.bsound");
        data.Values["gone"] = new PropertyValue.Flag(true);
        var read = data.Resolved(shape);
        Assert.Equal(new PropertyValue.Number(40), read["health"].Value);
        Assert.Equal(new PropertyValue.List([new PropertyValue.Number(3)]), read["waves"].Value);
        Assert.Equal(new PropertyValue.Text(""), read["drop"].Value);
        Assert.False(read.ContainsKey("gone"), "only the shape's fields are read");
        Assert.Equal(2, data.Problems(shape).Count);
        Assert.Equal(shape, DataShape.FromText(shape.ToText()).Value);
        Assert.Equal(data, DataFile.FromText(data.ToText()).Value);
    }

    [Fact]
    public void ShapesRefuseFieldsThatCannotBeReadAndLayGroupsOutTogether()
    {
        static DataField Field(string name) => DataField.Of(name, PropertyType.Number, PropertyShape.One);
        Assert.True(new DataShape([Field("a"), Field("a")]).Validate().IsError, "two fields of one name");
        Assert.True(new DataShape([Field("a") with { Default = new PropertyValue.Text("x") }]).Validate().IsError, "a default of another type");
        Assert.True(new DataShape([DataField.Of("name", PropertyType.Text, PropertyShape.One) with { Range = (0, 1) }]).Validate().IsError, "a range on text");
        Assert.True(new DataShape([Field("a") with { Code = "lists:number" }]).Validate().IsError, "a type code that names nothing");

        static DataField Grouped(string name) => Field(name) with { Group = "Combat" };
        var layout = new DataShape([Grouped("damage"), Field("speed"), Grouped("range")]).Layout();
        Assert.Equal(2, layout.Count);
        Assert.Equal(2, Assert.IsType<InspectorItem.Group>(layout[0]).Items.Count);
    }

    [Fact]
    public void FilePropertiesHoldPathsOfTheirKindOrNothing()
    {
        static PropertyValue Path(string t) => new PropertyValue.Text(t);
        Assert.True(Path("").Fits(PropertyType.Data));
        Assert.True(Path("data/Goblin.BDATA").Fits(PropertyType.Data), "the extension in any case");
        Assert.False(Path("data/goblin.bshape").Fits(PropertyType.Data));
        Assert.True(Path("sounds/hit.ogg").Fits(PropertyType.Sound));
        Assert.False(Path("sounds/hit.ogg").Fits(PropertyType.Prefab));
        Assert.False(Path(new string('a', 600) + ".bprefab").Fits(PropertyType.Prefab), "too long a path");
        foreach (var type in new[] { PropertyType.Prefab, PropertyType.Sound, PropertyType.Material, PropertyType.Data })
        {
            Assert.Equal(type, PropertyTypes.FromCode(type.Code()));
            Assert.True(type.Zero().Fits(type));
        }
    }
}

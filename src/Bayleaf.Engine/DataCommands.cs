using Bayleaf.Core.Json;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

namespace Bayleaf.Engine;

/// <summary>
/// Data files and their shapes: <c>data.shape.get</c>/<c>data.shape.set</c> read and write a shape
/// (<c>.bshape</c>), <c>data.get</c>/<c>data.set</c> a data file (<c>.bdata</c>), and <c>data.field.set</c>
/// one value of a data file.
/// </summary>
internal static class DataCommands
{
    public static IEnumerable<CommandSpec> All() =>
    [
        Spec("data.shape.get", "Read Data Shape",
            "{path}: a data shape (.bshape, relative to the project folder or absolute); returns {shape: {fields: [{name, type, default, label?, hint?, range?, slider?, group?}]}}. type is as scripts declare property types: number, whole, bool, vector, color, text, object, prefab, sound, material, data, or list:<type>, map:<type> (text keys), map:<key type>:<type>",
            ShapeGet),
        Spec("data.shape.set", "Change Data Shape",
            "{path, shape}: writes a data shape (all of it, as data.shape.get returns it); every data file that names it reads its fields from then on: a value that no longer fits reads as the field's default and is reported by data.get",
            ShapeSet),
        Spec("data.get", "Read Data File",
            "{path}: a data file (.bdata); returns {data: {shape, values}, fields: [{name, type, value, set}] (every field of its shape as it reads, set when the file gives the value), problems: [text]}",
            DataGet),
        Spec("data.set", "Change Data File",
            "{path, data: {shape, values}}: writes a data file (all of it): the shape it names (.bshape, relative to the project folder; empty for none) and the values that differ from the shape's defaults, by field name",
            DataSet),
        Spec("data.field.set", "Change Data Field",
            "{path, name, value}: sets one field of a data file to a value that fits the field in its shape (as data.get shows values); value null returns it to the shape's default. Refuses names the shape lacks",
            FieldSet),
    ];

    private static CommandSpec Spec(string id, string label, string parameters, Func<Session, JsonValue, JsonValue> run) =>
        new() { Id = id, Label = label, Params = parameters, Run = run };

    private static string PathParam(JsonValue p) => Params.Of(p, "path").Required("path", JsonRead.Str);

    private static JsonValue ShapeGet(Session session, JsonValue p)
    {
        var shape = DataShape.Load(LightingCommands.KindPath(session, PathParam(p), DataShape.Extension, Access.Read)).OrFail();
        return J.Obj(("shape", shape.ToJson()));
    }

    private static JsonValue ShapeSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "shape");
        var given = f.Required("path", JsonRead.Str);
        var shape = f.Required("shape", DataShape.FromJson);
        shape.Validate().OrBadParams();
        var path = LightingCommands.KindPath(session, given, DataShape.Extension, Access.Write);
        shape.Save(path).OrFail();
        return J.Obj(("path", J.S(path)));
    }

    /// <summary>The shape a data file names, as the session may read it; the error says why it cannot be.</summary>
    internal static Core.Result<DataShape, string> ShapeOf(Session session, DataFile data)
    {
        if (data.Shape.Length == 0)
        {
            return Core.Result.Fail("no shape is chosen");
        }
        string path;
        try
        {
            path = LightingCommands.KindPath(session, data.Shape, DataShape.Extension, Access.Read);
        }
        catch (CommandException e)
        {
            return Core.Result.Fail(e.Error.Message);
        }
        var shape = DataShape.Load(path);
        return shape.IsOk ? shape.Value : Core.Result.Fail($"its shape {data.Shape} does not read: {shape.Error.Message}");
    }

    private static JsonValue DataGet(Session session, JsonValue p)
    {
        var data = DataFile.Load(LightingCommands.KindPath(session, PathParam(p), DataFile.Extension, Access.Read)).OrFail();
        var fields = new JsonArray();
        IReadOnlyList<string> problems;
        var shape = ShapeOf(session, data);
        if (shape.IsOk)
        {
            var read = data.Resolved(shape.Value);
            foreach (var field in shape.Value.Fields)
            {
                if (read.TryGetValue(field.Name, out var r))
                {
                    var set = data.Values.TryGetValue(field.Name, out var own) && own.Equals(r.Value);
                    fields.Add(J.Obj(("name", J.S(field.Name)), ("type", J.S(field.Code)), ("value", r.Value.Write()), ("set", J.B(set))));
                }
            }
            problems = data.Problems(shape.Value);
        }
        else
        {
            problems = [shape.Error];
        }
        return J.Obj(("data", data.ToJson()), ("fields", fields), ("problems", J.Strings(problems)));
    }

    private static JsonValue DataSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "data");
        var given = f.Required("path", JsonRead.Str);
        var data = f.Required("data", DataFile.FromJson);
        data.Validate().OrBadParams();
        var path = LightingCommands.KindPath(session, given, DataFile.Extension, Access.Write);
        data.Save(path).OrFail();
        return J.Obj(("path", J.S(path)));
    }

    private static JsonValue FieldSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "name", "value");
        var given = f.Required("path", JsonRead.Str);
        var name = f.Required("name", JsonRead.Str);
        if (!f.Has("value"))
        {
            throw new JsonShapeException("missing field `value`");
        }
        var value = f.Nullable("value", PropertyValue.Read);
        var path = LightingCommands.KindPath(session, given, DataFile.Extension, Access.Write);
        var data = DataFile.Load(path).OrFail();
        var shape = ShapeOf(session, data);
        if (shape.IsError)
        {
            throw Stop.Failed($"cannot set a field: {shape.Error}");
        }
        var field = shape.Value.Field(name) ?? throw Stop.BadParams($"the shape {data.Shape} has no field {name}");
        if (value is null)
        {
            data.Values.Remove(name);
        }
        else
        {
            var declared = field.Declared() ?? throw Stop.Failed($"field {name} has an unknown type");
            if (!declared.Accepts(value))
            {
                throw Stop.BadParams($"{name} holds {field.Code}; that value is not one");
            }
            data.Values[name] = value;
        }
        data.Save(path).OrFail();
        return J.Obj(("path", J.S(path)));
    }
}

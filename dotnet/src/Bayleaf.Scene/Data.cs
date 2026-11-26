using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

namespace Bayleaf.Scene;

// Data files (.bdata) and their shapes (.bshape): plain data that scripts and scenes share instead of
// copying the same values into every object. A shape declares fields, each with a name, a type and a
// default, plus how the Inspector shows it. A data file names its shape and keeps the values that
// differ from the defaults; a field it leaves out, or a value that no longer fits its field, reads as
// the default (and the misfit is reported). Scripts read data files as they were saved.

/// <summary>One field of a shape.</summary>
public sealed record DataField
{
    /// <summary>1 to 64 bytes, no <c>:</c>; unique in the shape.</summary>
    public string Name { get; init; } = "";

    /// <summary>The type as scripts declare property types (<c>number</c>, <c>list:number</c>, <c>map:text:whole</c>).</summary>
    public string Code { get; init; } = "";

    /// <summary>The value a data file reads when it does not set this field.</summary>
    public PropertyValue Default { get; init; } = new PropertyValue.Number(0);

    /// <summary>Shown instead of the name.</summary>
    public string? Label { get; init; }

    /// <summary>Said on hover and to screen readers.</summary>
    public string Hint { get; init; } = "";

    /// <summary>The lowest and highest value a number may be set to.</summary>
    public (double Low, double High)? Range { get; init; }

    /// <summary>Shown as a slider across the range.</summary>
    public bool Slider { get; init; }

    /// <summary>The group the field shows in; fields of one group show together, in the order of the first.</summary>
    public string? Group { get; init; }

    /// <summary>A field of this type and shape with the type's zero as its default.</summary>
    public static DataField Of(string name, PropertyType type, PropertyShape shape) => new() { Name = name, Code = TypeCode(type, shape), Default = type.ZeroOf(shape) };

    /// <summary>The field's type and shape; null for a code that names none.</summary>
    public (PropertyType Type, PropertyShape Shape)? Kind() => PropertyShape.Parse(Code);

    /// <summary>The field as a declared property.</summary>
    public DeclaredProperty? Declared() => Kind() is (var type, var shape) ? new DeclaredProperty(Name, type, shape, Default) : null;

    /// <summary>The code a type and shape are declared with (<c>number</c>, <c>list:number</c>, <c>map:whole:number</c>).</summary>
    public static string TypeCode(PropertyType type, PropertyShape shape) => shape.Kind switch
    {
        ShapeKind.One => type.Code(),
        ShapeKind.List => $"list:{type.Code()}",
        _ when shape.KeyType == PropertyType.Text => $"map:{type.Code()}",
        _ => $"map:{shape.KeyType.Code()}:{type.Code()}",
    };

    internal Result<Unit, string> Validate()
    {
        if (Name.Length == 0 || Text.Bytes(Name) > DataShape.MaxName || Name.Contains(':', StringComparison.Ordinal) || Text.HasControl(Name))
        {
            return Checks.Fail($"field name {Text.Debug(Name)} must be 1 to {DataShape.MaxName} bytes without : or control characters");
        }
        if (Kind() is not (var type, var shape))
        {
            return Checks.Fail($"field {Name} has an unknown type {Text.Debug(Code)}");
        }
        if (!Default.FitsShape(type, shape))
        {
            return Checks.Fail($"field {Name}'s default does not fit its type {Code}");
        }
        Result<Unit, string> Short(string what, string text, int max) =>
            Text.Bytes(text) <= max && !Text.HasControl(text) ? Checks.Ok : Checks.Fail($"field {Name}'s {what} is longer than {max} bytes");
        var texts = Checks.All(() => Short("label", Label ?? "", DataShape.MaxName), () => Short("hint", Hint, DataShape.MaxHint), () => Short("group", Group ?? "", DataShape.MaxName));
        if (texts.IsError)
        {
            return texts;
        }
        if (Range is (var low, var high))
        {
            if (type is not (PropertyType.Number or PropertyType.WholeNumber) || shape != PropertyShape.One)
            {
                return Checks.Fail($"field {Name} has a range, but only a number or whole number takes one");
            }
            if (!(double.IsFinite(low) && double.IsFinite(high) && low < high))
            {
                return Checks.Fail($"field {Name}'s range must be two finite numbers, the lower first");
            }
        }
        return Slider && Range is null ? Checks.Fail($"field {Name} is a slider without a range") : Checks.Ok;
    }

    internal static DataField Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct DataField", true, "name", "type", "default", "label", "hint", "range", "slider", "group");
        return new()
        {
            Name = f.Required("name", JsonRead.Str),
            Code = f.Required("type", JsonRead.Str),
            Default = f.Required("default", PropertyValue.Read),
            Label = f.Nullable("label", JsonRead.Str),
            Hint = f.Optional("hint", JsonRead.Str, ""),
            Range = f.Nullable("range", v => JsonRead.Fixed(v, 2, JsonRead.F64)) is [var low, var high] ? (low, high) : null,
            Slider = f.Optional("slider", JsonRead.Bool, false),
            Group = f.Nullable("group", JsonRead.Str),
        };
    }

    internal JsonValue Write()
    {
        var o = new JsonObject { { "name", J.S(Name) }, { "type", J.S(Code) }, { "default", Default.Write() } };
        if (Label is not null)
        {
            o.Add("label", J.S(Label));
        }
        if (Hint.Length > 0)
        {
            o.Add("hint", J.S(Hint));
        }
        if (Range is (var low, var high))
        {
            o.Add("range", J.Doubles(low, high));
        }
        if (Slider)
        {
            o.Add("slider", J.B(true));
        }
        if (Group is not null)
        {
            o.Add("group", J.S(Group));
        }
        return o;
    }
}

/// <summary>The fields a data file has: what the Inspector shows and what scripts read.</summary>
public sealed record DataShape(IReadOnlyList<DataField> Fields)
{
    public const string Extension = "bshape";
    public const string Format = "bayleaf.shape";
    public const uint Version = 1;

    /// <summary>Most fields a shape declares.</summary>
    public const int MaxFields = 256;

    /// <summary>Largest shape or data file read, in bytes.</summary>
    public const long MaxFileBytes = 4 << 20;

    /// <summary>Longest field name, label or group title, in bytes.</summary>
    public const int MaxName = 64;

    /// <summary>Longest hint, in bytes.</summary>
    public const int MaxHint = 512;

    public bool Equals(DataShape? other) => other is not null && Fields.SequenceEqual(other.Fields);

    public override int GetHashCode() => Fields.Count;

    /// <summary>The shape a new shape file starts with: one number field.</summary>
    public static DataShape Starter() => new([DataField.Of("value", PropertyType.Number, PropertyShape.One)]);

    /// <summary>Checks every field and that names are unique.</summary>
    public Result<Unit, string> Validate()
    {
        if (Fields.Count > MaxFields)
        {
            return Checks.Fail($"a shape has at most {MaxFields} fields");
        }
        for (var i = 0; i < Fields.Count; i++)
        {
            var valid = Fields[i].Validate();
            if (valid.IsError)
            {
                return valid;
            }
            if (Fields.Take(i).Any(f => f.Name == Fields[i].Name))
            {
                return Checks.Fail($"two fields are named {Fields[i].Name}");
            }
        }
        return Checks.Ok;
    }

    public DataField? Field(string name) => Fields.FirstOrDefault(f => f.Name == name);

    /// <summary>Every field as a declared property, in order.</summary>
    public IReadOnlyList<DeclaredProperty> Declared() => Fields.Select(f => f.Declared()).OfType<DeclaredProperty>().ToList();

    /// <summary>How the Inspector lays the fields out: one row each, those of a group together in a group placed where its first field is.</summary>
    public IReadOnlyList<InspectorItem> Layout()
    {
        static InspectorItem Row(DataField f) => new InspectorItem.Row(new PropertyLayout { Name = f.Name, Label = f.Label, Hint = f.Hint, Range = f.Range, Slider = f.Slider });
        var items = new List<InspectorItem>();
        var placed = new List<string>();
        foreach (var field in Fields)
        {
            var group = string.IsNullOrEmpty(field.Group) ? null : field.Group;
            if (group is null)
            {
                items.Add(Row(field));
            }
            else if (!placed.Contains(group))
            {
                placed.Add(group);
                items.Add(new InspectorItem.Group(group, false, Fields.Where(f => f.Group == group).Select(Row).ToList(), null));
            }
        }
        return items;
    }

    public string ToText() => new JsonObject { { "format", J.S(Format) }, { "version", J.U(Version) }, { "fields", new JsonArray(Fields.Select(f => f.Write())) } }.ToPretty() + "\n";

    public static Result<DataShape, SceneError> FromText(string text)
    {
        var read = JsonReader.Parse(text).TryGet(out var value, out var parseError)
            ? JsonRead.Try(() =>
            {
                var f = JsonFields.Of(value, "struct ShapeFile", true, "format", "version", "fields");
                return (Format: f.Required("format", JsonRead.Str), Version: f.Required("version", JsonRead.U32),
                    Fields: f.Optional("fields", v => (IReadOnlyList<DataField>)JsonRead.List(v, DataField.Read), []));
            })
            : Result.Fail(parseError);
        if (!read.TryGet(out var file, out var error))
        {
            return Result.Fail(SceneError.Format(error));
        }
        if (file.Format != Format)
        {
            return Result.Fail(SceneError.Format($"not a data shape (format \"{file.Format}\")"));
        }
        if (file.Version is < 1 or > Version)
        {
            return Result.Fail(SceneError.Format($"unsupported data shape version {file.Version}"));
        }
        var shape = new DataShape(file.Fields);
        var valid = shape.Validate();
        return valid.IsOk ? shape : Result.Fail(SceneError.Format(valid.Error));
    }

    public static Result<DataShape, SceneError> Load(string path) => DataFile.ReadLimited(path).TryGet(out var text, out var error) ? FromText(text) : Result.Fail(error);

    /// <summary>Writes the file through a temporary one, so a failed write keeps the old file.</summary>
    public Result<Unit, SceneError> Save(string path)
    {
        var valid = Validate();
        return valid.IsError ? Result.Fail(SceneError.Format(valid.Error)) : Files.WriteReplacing(path, path + ".tmp", ToText()).MapError(SceneError.Io);
    }
}

/// <summary>One field of a data file as it reads: its type, shape and value.</summary>
public sealed record ResolvedField(PropertyType Type, PropertyShape Shape, PropertyValue Value);

/// <summary>A data file: its shape and the values it sets.</summary>
public sealed record DataFile(string Shape, SortedDictionary<string, PropertyValue> Values)
{
    public const string Extension = "bdata";
    public const string Format = "bayleaf.data";
    public const uint Version = 1;

    public DataFile(string shape)
        : this(shape, new SortedDictionary<string, PropertyValue>(Utf8Order.Instance))
    {
    }

    public bool Equals(DataFile? other) => other is not null && Shape == other.Shape && Values.Count == other.Values.Count
        && Values.Zip(other.Values).All(p => p.First.Key == p.Second.Key && p.First.Value.Equals(p.Second.Value));

    public override int GetHashCode() => StringComparer.Ordinal.GetHashCode(Shape);

    /// <summary>Checks the shape's path and the values' names and sizes (not that they fit the shape, which may change after the file is written).</summary>
    public Result<Unit, string> Validate()
    {
        if (Shape.Length > 0 && !(Text.Bytes(Shape) <= 512 && Text.AsciiLower(Shape).EndsWith($".{DataShape.Extension}", StringComparison.Ordinal)))
        {
            return Checks.Fail($"shape must be the path of a .{DataShape.Extension} file (at most 512 characters)");
        }
        if (Values.Count > DataShape.MaxFields)
        {
            return Checks.Fail($"a data file sets at most {DataShape.MaxFields} values");
        }
        foreach (var (name, value) in Values)
        {
            if (name.Length == 0 || Text.Bytes(name) > DataShape.MaxName || name.Contains(':', StringComparison.Ordinal))
            {
                return Checks.Fail($"value name {Text.Debug(name)} must be 1 to {DataShape.MaxName} bytes without :");
            }
            if (!PropertyShape.All().Any(s => PropertyTypes.All.Any(t => value.FitsShape(t, s))))
            {
                return Checks.Fail($"value {name} is not a value any field holds");
            }
        }
        return Checks.Ok;
    }

    /// <summary>Every field of <paramref name="shape"/> as this file reads: its own value where it sets one that fits, else the default.</summary>
    public SortedDictionary<string, ResolvedField> Resolved(DataShape shape)
    {
        var resolved = new SortedDictionary<string, ResolvedField>(Utf8Order.Instance);
        foreach (var field in shape.Fields)
        {
            if (field.Kind() is (var type, var kind))
            {
                var value = Values.TryGetValue(field.Name, out var own) && own.FitsShape(type, kind) ? own : field.Default;
                resolved[field.Name] = new ResolvedField(type, kind, value);
            }
        }
        return resolved;
    }

    /// <summary>What does not read as written against <paramref name="shape"/>: values of fields the shape lacks, and values that no longer fit their field.</summary>
    public IReadOnlyList<string> Problems(DataShape shape)
    {
        var problems = new List<string>();
        foreach (var (name, value) in Values)
        {
            var field = shape.Field(name);
            if (field?.Kind() is not (var type, var kind))
            {
                problems.Add($"{name} is not a field of the shape; it is kept but not read");
            }
            else if (!value.FitsShape(type, kind))
            {
                problems.Add($"{name} does not fit its field's type {field.Code}; it reads as the default");
            }
        }
        return problems;
    }

    public string ToText() => new JsonObject
    {
        { "format", J.S(Format) },
        { "version", J.U(Version) },
        { "shape", J.S(Shape) },
        { "values", new JsonObject(Values.Select(v => new KeyValuePair<string, JsonValue>(v.Key, v.Value.Write()))) },
    }.ToPretty() + "\n";

    public static Result<DataFile, SceneError> FromText(string text)
    {
        var read = JsonReader.Parse(text).TryGet(out var value, out var parseError)
            ? JsonRead.Try(() =>
            {
                var f = JsonFields.Of(value, "struct DataText", true, "format", "version", "shape", "values");
                return (Format: f.Required("format", JsonRead.Str), Version: f.Required("version", JsonRead.U32), Shape: f.Required("shape", JsonRead.Str),
                    Values: f.Optional("values", v => JsonRead.Map(v, PropertyValue.Read), new SortedDictionary<string, PropertyValue>(Utf8Order.Instance)));
            })
            : Result.Fail(parseError);
        if (!read.TryGet(out var file, out var error))
        {
            return Result.Fail(SceneError.Format(error));
        }
        if (file.Format != Format)
        {
            return Result.Fail(SceneError.Format($"not a data file (format \"{file.Format}\")"));
        }
        if (file.Version is < 1 or > Version)
        {
            return Result.Fail(SceneError.Format($"unsupported data file version {file.Version}"));
        }
        var data = new DataFile(file.Shape, file.Values);
        var valid = data.Validate();
        return valid.IsOk ? data : Result.Fail(SceneError.Format(valid.Error));
    }

    public static Result<DataFile, SceneError> Load(string path) => ReadLimited(path).TryGet(out var text, out var error) ? FromText(text) : Result.Fail(error);

    /// <summary>Writes the file through a temporary one, so a failed write keeps the old file.</summary>
    public Result<Unit, SceneError> Save(string path)
    {
        var valid = Validate();
        return valid.IsError ? Result.Fail(SceneError.Format(valid.Error)) : Files.WriteReplacing(path, path + ".tmp", ToText()).MapError(SceneError.Io);
    }

    internal static Result<string, SceneError> ReadLimited(string path)
    {
        var length = Files.Length(path);
        if (length.IsError)
        {
            return Result.Fail(SceneError.Io($"cannot read {path}: {length.Error}"));
        }
        if (length.Value > DataShape.MaxFileBytes)
        {
            return Result.Fail(SceneError.Format($"{path} is larger than {DataShape.MaxFileBytes >> 20} MiB"));
        }
        return Files.ReadText(path).MapError(e => SceneError.Io($"cannot read {path}: {e}"));
    }
}

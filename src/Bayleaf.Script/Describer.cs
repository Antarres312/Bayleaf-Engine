using System.Globalization;
using System.Numerics;

using Bayleaf.Core.Json;
using Bayleaf.Graph;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

using SceneProperty = Bayleaf.Scene.Components.DeclaredProperty;

namespace Bayleaf.Script;

/// <summary>An event a script declares: its name and its values, each <c>name:type</c>.</summary>
public sealed record DeclaredEvent(string Name, IReadOnlyList<string> Values);

/// <summary>One property of a C# script: what the scene and Inspector see, and the field the engine sets.</summary>
public sealed record ScriptField(SceneProperty Declared, string Field);

/// <summary>
/// What a C# script declares, read from its source by the compiler (no script code runs): its
/// properties with their types and defaults, how the Inspector lays them out, where Add Component
/// lists it and the events it triggers. A script with problems does not run.
/// </summary>
public sealed record ScriptDescription(
    string File,
    string TypeName,
    IReadOnlyList<ScriptField> Fields,
    string? Menu,
    IReadOnlyList<InspectorItem> Inspector,
    IReadOnlyList<DeclaredEvent> Events,
    bool StepsMany,
    IReadOnlyList<string> Problems)
{
    public IReadOnlyList<SceneProperty> Properties => [.. Fields.Select(f => f.Declared)];
}

/// <summary>Reads each script file's script from a compilation.</summary>
internal static class Describer
{
    /// <summary>Most events one script declares, and most values one event brings.</summary>
    public const int MaxEvents = 64;

    public const int MaxEventValues = 16;

    /// <summary>Longest message or event name, in bytes.</summary>
    public const int MaxName = 64;

    public static Dictionary<string, ScriptDescription> All(CSharpCompilation compilation, List<ScriptProblem> problems)
    {
        var output = new Dictionary<string, ScriptDescription>(StringComparer.Ordinal);
        var baseType = compilation.GetTypeByMetadataName("Bayleaf.Script.Sdk.GameScript");
        var many = compilation.GetTypeByMetadataName("Bayleaf.Script.Sdk.IAdvanceMany`1");
        if (baseType is null)
        {
            return output;
        }
        foreach (var tree in compilation.SyntaxTrees)
        {
            if (tree.FilePath.StartsWith('<'))
            {
                continue;
            }
            var model = compilation.GetSemanticModel(tree);
            var scripts = tree.GetRoot().DescendantNodes().OfType<ClassDeclarationSyntax>()
                .Select(c => model.GetDeclaredSymbol(c))
                .OfType<INamedTypeSymbol>()
                .Where(t => !t.IsAbstract && Derives(t, baseType))
                .Distinct<INamedTypeSymbol>(SymbolEqualityComparer.Default)
                .ToList();
            if (scripts.Count == 0)
            {
                continue;
            }
            if (scripts.Count > 1)
            {
                problems.Add(new ScriptProblem(tree.FilePath, 0, 0, $"declares {scripts.Count} scripts ({string.Join(", ", scripts.Select(s => s.Name))}); a script file holds one"));
                continue;
            }
            var description = Describe(tree.FilePath, scripts[0], model, many);
            problems.AddRange(description.Problems.Select(p => new ScriptProblem(tree.FilePath, 0, 0, p)));
            output[tree.FilePath] = description;
        }
        return output;
    }

    private static bool Derives(INamedTypeSymbol type, INamedTypeSymbol baseType)
    {
        for (var t = type.BaseType; t is not null; t = t.BaseType)
        {
            if (SymbolEqualityComparer.Default.Equals(t, baseType))
            {
                return true;
            }
        }
        return false;
    }

    private static ScriptDescription Describe(string file, INamedTypeSymbol type, SemanticModel model, INamedTypeSymbol? many)
    {
        var problems = new List<string>();
        if (type.DeclaredAccessibility != Accessibility.Public || type.ContainingType is not null && type.ContainingType.DeclaredAccessibility != Accessibility.Public)
        {
            problems.Add($"{type.Name} must be public for the engine to run it");
        }
        if (type.IsGenericType)
        {
            problems.Add($"{type.Name} must not be generic");
        }
        if (!type.InstanceConstructors.Any(c => c.Parameters.Length == 0 && c.DeclaredAccessibility == Accessibility.Public))
        {
            problems.Add($"{type.Name} needs a public constructor that takes nothing");
        }
        var fields = new List<ScriptField>();
        var layout = new JsonArray();
        foreach (var field in Members(type).OfType<IFieldSymbol>())
        {
            var attribute = field.GetAttributes().FirstOrDefault(a => a.AttributeClass?.ToDisplayString() == "Bayleaf.Script.Sdk.ScriptPropertyAttribute");
            if (attribute is null)
            {
                continue;
            }
            if (Field(field, attribute, model, problems) is { } made)
            {
                if (fields.Any(f => f.Declared.Name == made.Field.Declared.Name))
                {
                    problems.Add($"property {Core.Text.Debug(made.Field.Declared.Name)} is declared twice");
                    continue;
                }
                fields.Add(made.Field);
                if (made.Layout is { } item)
                {
                    layout.Add(item);
                }
            }
        }
        var declared = fields.Select(f => f.Declared).ToList();
        IReadOnlyList<InspectorItem> inspector = [];
        if (layout.Count > 0)
        {
            var parsed = InspectorItem.Parse(layout, declared);
            if (parsed.IsError)
            {
                problems.Add($"inspector: {parsed.Error}");
            }
            else
            {
                inspector = parsed.Value;
            }
        }
        string? menu = null;
        if (type.GetAttributes().FirstOrDefault(a => a.AttributeClass?.ToDisplayString() == "Bayleaf.Script.Sdk.ScriptMenuAttribute") is { } menuAttribute)
        {
            var path = menuAttribute.ConstructorArguments.FirstOrDefault().Value as string ?? "";
            var checkedMenu = Visual.MenuPath(path);
            if (checkedMenu.IsError)
            {
                problems.Add(checkedMenu.Error);
            }
            else
            {
                menu = checkedMenu.Value;
            }
        }
        var events = Events(type, problems);
        var stepsMany = many is not null && type.AllInterfaces.Any(i => SymbolEqualityComparer.Default.Equals(i.OriginalDefinition, many));
        var typeName = type.ContainingType is null ? type.ToDisplayString(MetadataName) : MetadataNameOf(type);
        return new ScriptDescription(file, typeName, fields, menu, inspector, events, stepsMany, problems);
    }

    private static readonly SymbolDisplayFormat MetadataName = new(typeQualificationStyle: SymbolDisplayTypeQualificationStyle.NameAndContainingTypesAndNamespaces);

    private static string MetadataNameOf(INamedTypeSymbol type)
    {
        var names = new List<string>();
        for (var t = type; t is not null; t = t.ContainingType)
        {
            names.Insert(0, t.MetadataName);
        }
        var space = type.ContainingNamespace is { IsGlobalNamespace: false } ns ? ns.ToDisplayString() + "." : "";
        return space + string.Join('+', names);
    }

    /// <summary>The type's members and its own script base classes' (not the SDK's).</summary>
    private static IEnumerable<ISymbol> Members(INamedTypeSymbol type)
    {
        var chain = new List<INamedTypeSymbol>();
        for (var t = type; t is not null && t.ContainingAssembly.Name != "Bayleaf.Script.Sdk" && t.SpecialType != SpecialType.System_Object; t = t.BaseType)
        {
            chain.Insert(0, t);
        }
        return chain.SelectMany(t => t.GetMembers());
    }

    private static T? Named<T>(AttributeData attribute, string name) =>
        attribute.NamedArguments.FirstOrDefault(a => a.Key == name).Value.Value is T value ? value : default;

    private static bool HasNamed(AttributeData attribute, string name) => attribute.NamedArguments.Any(a => a.Key == name);

    private static (ScriptField Field, JsonValue? Layout)? Field(IFieldSymbol field, AttributeData attribute, SemanticModel model, List<string> problems)
    {
        var name = attribute.ConstructorArguments.FirstOrDefault().Value as string;
        name ??= field.Name.Length > 0 ? char.ToLowerInvariant(field.Name[0]) + field.Name[1..] : field.Name;
        if (name.Length == 0 || Core.Text.Bytes(name) > 64 || name.Contains(':', StringComparison.Ordinal))
        {
            problems.Add($"property {Core.Text.Debug(name)} needs a name of 1 to 64 bytes without ':'");
            return null;
        }
        if (field.IsStatic || field.IsConst || field.IsReadOnly)
        {
            problems.Add($"property {name}: {field.Name} must be a field of each script that the engine can set (not static, const or readonly)");
            return null;
        }
        var color = Named<bool>(attribute, "Color");
        var file = Named<string>(attribute, "File");
        PropertyType? type = (field.Type.SpecialType, field.Type.ToDisplayString()) switch
        {
            (SpecialType.System_Single or SpecialType.System_Double, _) => PropertyType.Number,
            (SpecialType.System_Int32, _) => PropertyType.WholeNumber,
            (SpecialType.System_Boolean, _) => PropertyType.TrueFalse,
            (_, "System.Numerics.Vector3") => color ? PropertyType.Color : PropertyType.Vector,
            (SpecialType.System_String, _) => file switch
            {
                null => PropertyType.Text,
                "prefab" => PropertyType.Prefab,
                "sound" => PropertyType.Sound,
                "material" => PropertyType.Material,
                "data" => PropertyType.Data,
                _ => null,
            },
            (_, "Bayleaf.Script.Sdk.SceneObject") => PropertyType.SceneObject,
            _ => null,
        };
        if (type is not { } t)
        {
            problems.Add(file is not null && field.Type.SpecialType == SpecialType.System_String
                ? $"property {name}: File is prefab, sound, material or data, not {Core.Text.Debug(file)}"
                : $"property {name}: {field.Name} is a {field.Type.ToDisplayString()}; a script property is a float, double, int, bool, Vector3, string or SceneObject");
            return null;
        }
        if (color && t != PropertyType.Color || file is not null && !t.IsFile())
        {
            problems.Add($"property {name}: Color is for a Vector3 and File for a string");
            return null;
        }
        var initial = Initial(field, model, t);
        if (initial.Problem is { } problem)
        {
            problems.Add($"property {name}: {problem}");
            return null;
        }
        var value = initial.Value ?? t.Zero();
        if (!value.Fits(t))
        {
            problems.Add($"property {name}: its initial value {value.Write().ToCompact()} is not a {t.Label()}");
            return null;
        }
        var declared = new SceneProperty(name, t, PropertyShape.One, value);
        JsonObject? item = null;
        void Option(string key, JsonValue v) => (item ??= new JsonObject { { "property", new JsonString(name) } }).Add(key, v);
        if (Named<string>(attribute, "Label") is { } label)
        {
            Option("label", new JsonString(label));
        }
        if (Named<string>(attribute, "Hint") is { } hint)
        {
            Option("hint", new JsonString(hint));
        }
        if (HasNamed(attribute, "Low") || HasNamed(attribute, "High"))
        {
            Option("range", new JsonArray([JsonNumber.Of(Named<double>(attribute, "Low")), JsonNumber.Of(Named<double>(attribute, "High"))]));
        }
        if (Named<bool>(attribute, "Slider"))
        {
            Option("slider", JsonValue.From(true));
        }
        if (Named<bool>(attribute, "ReadOnly"))
        {
            Option("readOnly", JsonValue.From(true));
        }
        if (Named<string>(attribute, "Choices") is { } choices)
        {
            Option("choices", new JsonArray(choices.Split('|').Select(c => (JsonValue)new JsonString(c.Trim()))));
        }
        return (new ScriptField(declared, field.Name), item);
    }

    /// <summary>The field's initial value as the property's default: a constant, or <c>new Vector3(x, y, z)</c> of constants; none without an initializer.</summary>
    private static (PropertyValue? Value, string? Problem) Initial(IFieldSymbol field, SemanticModel model, PropertyType type)
    {
        var declarator = field.DeclaringSyntaxReferences.Select(r => r.GetSyntax()).OfType<VariableDeclaratorSyntax>().FirstOrDefault();
        if (declarator?.Initializer?.Value is not { } expression)
        {
            return (null, null);
        }
        var fieldModel = model.SyntaxTree == expression.SyntaxTree ? model : model.Compilation.GetSemanticModel(expression.SyntaxTree);
        const string NotConstant = "its initial value, the default, must be a constant (or new Vector3 of constants)";
        if (type is PropertyType.Vector or PropertyType.Color)
        {
            var arguments = expression switch
            {
                ObjectCreationExpressionSyntax o => o.ArgumentList?.Arguments,
                ImplicitObjectCreationExpressionSyntax i => i.ArgumentList.Arguments,
                _ => null,
            };
            if (expression is MemberAccessExpressionSyntax { Name.Identifier.Text: var member })
            {
                return member switch
                {
                    "Zero" => (new PropertyValue.Triple(Vector3.Zero), null),
                    "One" => (new PropertyValue.Triple(Vector3.One), null),
                    "UnitX" => (new PropertyValue.Triple(Vector3.UnitX), null),
                    "UnitY" => (new PropertyValue.Triple(Vector3.UnitY), null),
                    "UnitZ" => (new PropertyValue.Triple(Vector3.UnitZ), null),
                    _ => (null, NotConstant),
                };
            }
            if (arguments is not { } args || args.Count is not (1 or 3))
            {
                return (null, NotConstant);
            }
            var parts = new List<float>();
            foreach (var argument in args)
            {
                if (Number(fieldModel.GetConstantValue(argument.Expression)) is not { } n)
                {
                    return (null, NotConstant);
                }
                parts.Add((float)n);
            }
            return (new PropertyValue.Triple(parts.Count == 1 ? new Vector3(parts[0]) : new Vector3(parts[0], parts[1], parts[2])), null);
        }
        if (type == PropertyType.SceneObject)
        {
            return expression is LiteralExpressionSyntax { RawKind: (int)SyntaxKind.DefaultLiteralExpression } or DefaultExpressionSyntax ? (null, null) : (null, "an object property starts as no object (the scene names one)");
        }
        var constant = fieldModel.GetConstantValue(expression);
        if (!constant.HasValue)
        {
            return (null, NotConstant);
        }
        return (type, constant.Value) switch
        {
            (PropertyType.TrueFalse, bool b) => (new PropertyValue.Flag(b), null),
            (PropertyType.Number or PropertyType.WholeNumber, var v) when Number(constant) is { } n => (new PropertyValue.Number(type == PropertyType.Number && v is float f ? f : n), null),
            (_, string s) => (new PropertyValue.Text(s), null),
            (_, null) => (new PropertyValue.Text(""), null),
            _ => (null, NotConstant),
        };
    }

    private static double? Number(Optional<object?> constant) => constant.HasValue ? constant.Value switch
    {
        float f => f,
        double d => d,
        int i => i,
        long l => l,
        _ => null,
    } : null;

    private static List<DeclaredEvent> Events(INamedTypeSymbol type, List<string> problems)
    {
        var events = new List<DeclaredEvent>();
        foreach (var attribute in type.GetAttributes().Where(a => a.AttributeClass?.ToDisplayString() == "Bayleaf.Script.Sdk.ScriptEventAttribute"))
        {
            var arguments = attribute.ConstructorArguments;
            var name = arguments.Length > 0 ? arguments[0].Value as string ?? "" : "";
            var values = arguments.Length > 1 && arguments[1].Kind == TypedConstantKind.Array
                ? arguments[1].Values.Select(v => v.Value as string ?? "").ToList()
                : arguments.Skip(1).Select(v => v.Value as string ?? "").ToList();
            if (name.Length == 0 || Core.Text.Bytes(name) > MaxName || events.Any(e => e.Name == name))
            {
                problems.Add($"event {Core.Text.Debug(name)} needs a name of 1 to {MaxName} bytes, once");
                continue;
            }
            var fine = values.Count <= MaxEventValues && values.All(p => p.IndexOf(':', StringComparison.Ordinal) is var colon and > 0 && VariableTypes.FromShapedCode(p[(colon + 1)..]) is not null);
            if (!fine)
            {
                problems.Add($"event {name}: at most {MaxEventValues} values, each name:type");
                continue;
            }
            events.Add(new DeclaredEvent(name, values));
        }
        if (events.Count > MaxEvents)
        {
            problems.Add($"at most {MaxEvents} events");
        }
        return events;
    }

    internal static string Invariant(FormattableString text) => text.ToString(CultureInfo.InvariantCulture);
}

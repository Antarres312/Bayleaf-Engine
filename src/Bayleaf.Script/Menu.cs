using Bayleaf.Core;
using Bayleaf.Scene.Components;

using SceneProperty = Bayleaf.Scene.Components.DeclaredProperty;

namespace Bayleaf.Script;

/// <summary>Where Add Component lists a script, and the properties a visual script declares as the script host reads them.</summary>
public static partial class Visual
{
    /// <summary>Most names in a menu path.</summary>
    private const int MaxMenuDepth = 4;

    /// <summary>Most characters in one name of a menu path.</summary>
    private const int MaxMenuPart = 64;

    /// <summary>
    /// A menu path as Add Component lists it: 1 to 4 names of 1 to 64 characters separated by <c>/</c>,
    /// each trimmed (<c>Light/Flicker</c>); the error says what is wrong.
    /// </summary>
    public static Result<string, string> MenuPath(string text)
    {
        var parts = text.Split('/').Select(p => p.Trim()).ToList();
        var fine = parts.Count <= MaxMenuDepth && parts.All(p => p.Length > 0 && Text.Chars(p) <= MaxMenuPart && !p.Any(char.IsControl));
        return fine
            ? string.Join('/', parts)
            : Result.Fail($"menu {Text.Debug(text)} needs 1 to {MaxMenuDepth} names of 1 to {MaxMenuPart} characters separated by / (Light/Flicker)");
    }

    /// <summary>
    /// The properties the graph declares, in order, as the script host reads them from the compiled
    /// script's description: the scene's type and shape, and the default (the type's zero where the
    /// declared one does not fit).
    /// </summary>
    public static IReadOnlyList<SceneProperty> DeclaredProperties(Graph.Graph graph) => graph.Properties.Select(p =>
    {
        var (type, shape) = (ScenePropertyType(p.Type), ScenePropertyShape(p.Shape, p.Key));
        var value = PropertyValue.FromDescribed(DescribedValue(p.Type, p.Shape, p.Default), type, shape) ?? type.ZeroOf(shape);
        return new SceneProperty(p.Name, type, shape, value);
    }).ToList();
}

using Bayleaf.Core;

namespace Bayleaf.Scene;

public enum SceneErrorKind
{
    UnknownObject,
    UnknownComponent,
    MissingComponent,
    DuplicateComponent,
    NotRemovable,
    InvalidValue,
    Cycle,
    Format,
    Io,
    Internal,
}

/// <summary>A scene operation that could not be done. The message is shown to the user.</summary>
public sealed record SceneError(SceneErrorKind Kind, string Message)
{
    public static SceneError UnknownObject(StableId id) => new(SceneErrorKind.UnknownObject, $"no object with id {id}");

    public static SceneError UnknownComponent(string name) => new(SceneErrorKind.UnknownComponent, $"unknown component \"{name}\"");

    public static SceneError MissingComponent(StableId id, string component) => new(SceneErrorKind.MissingComponent, $"object {id} has no \"{component}\" component");

    public static SceneError DuplicateComponent(StableId id, string component) => new(SceneErrorKind.DuplicateComponent, $"object {id} already has a \"{component}\" component");

    public static SceneError NotRemovable(string component) => new(SceneErrorKind.NotRemovable, $"the \"{component}\" component cannot be removed");

    public static SceneError InvalidValue(string component, string message) => new(SceneErrorKind.InvalidValue, $"invalid \"{component}\" value: {message}");

    public static readonly SceneError Cycle = new(SceneErrorKind.Cycle, "an object cannot become a child of itself or of its own descendant");

    public static SceneError Format(string message) => new(SceneErrorKind.Format, $"scene file: {message}");

    public static SceneError Io(string message) => new(SceneErrorKind.Io, message);

    public static SceneError Internal(string message) => new(SceneErrorKind.Internal, message);

    public override string ToString() => Message;
}

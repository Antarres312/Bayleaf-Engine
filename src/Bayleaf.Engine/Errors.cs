using Bayleaf.Scene;

namespace Bayleaf.Engine;

public enum CommandErrorKind
{
    Unknown,
    Disabled,
    ParamsNotObject,
    BadParams,
    Failed,
}

/// <summary>Why a command did not run. The message is shown to users and agents, as the Rust engine words it.</summary>
public sealed record CommandError(CommandErrorKind Kind, string Message)
{
    public static CommandError Unknown(string id) => new(CommandErrorKind.Unknown, $"unknown command: {id}");

    public static CommandError Disabled(string id, string reason) => new(CommandErrorKind.Disabled, $"command {id} is disabled: {reason}");

    public static readonly CommandError ParamsNotObject = new(CommandErrorKind.ParamsNotObject, "params must be a JSON object, null or omitted");

    public static CommandError BadParams(string message) => new(CommandErrorKind.BadParams, $"invalid params: {message}");

    public static CommandError Failed(string message) => new(CommandErrorKind.Failed, message);

    public static CommandError From(SceneError error) => Failed(error.Message);

    public override string ToString() => Message;
}

/// <summary>
/// A command that stops with an error. Command code throws it; the registry catches it at its
/// boundary and returns the error, so it never leaves the engine.
/// </summary>
public sealed class CommandException : Exception
{
    public CommandException()
        : this(CommandError.Failed("the command failed"))
    {
    }

    public CommandException(string message)
        : this(CommandError.Failed(message))
    {
    }

    public CommandException(string message, Exception inner)
        : base(message, inner) => Error = CommandError.Failed(message);

    public CommandException(CommandError error)
        : base(error.Message) => Error = error;

    public CommandError Error { get; }
}

/// <summary>Short ways for command code to stop.</summary>
internal static class Stop
{
    public static CommandException Failed(string message) => new(CommandError.Failed(message));

    public static CommandException BadParams(string message) => new(CommandError.BadParams(message));

    /// <summary>The value, or stop with the scene's error.</summary>
    public static T OrFail<T>(this Core.Result<T, SceneError> result) => result.IsOk ? result.Value : throw new CommandException(CommandError.From(result.Error));

    /// <summary>The value, or stop with the message as a failure.</summary>
    public static T OrFail<T>(this Core.Result<T, string> result) => result.IsOk ? result.Value : throw Failed(result.Error);

    /// <summary>The value, or stop with the message as bad params.</summary>
    public static T OrBadParams<T>(this Core.Result<T, string> result) => result.IsOk ? result.Value : throw BadParams(result.Error);
}

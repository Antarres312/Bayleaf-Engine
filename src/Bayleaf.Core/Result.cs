using System.Diagnostics.CodeAnalysis;

namespace Bayleaf.Core;

/// <summary>The value of an operation that has nothing to give back.</summary>
public readonly record struct Unit
{
    public static readonly Unit Value;
}

/// <summary>A failure, before it knows the kind of value it stands in for: <c>return Result.Fail(e)</c>.</summary>
public readonly record struct Failure<TError>(TError Error);

/// <summary>Builds results. Expected failures are results, not exceptions.</summary>
public static class Result
{
    public static Failure<TError> Fail<TError>(TError error) => new(error);

    /// <summary>A unit result that succeeded.</summary>
    public static Result<Unit, TError> Ok<TError>() => Unit.Value;
}

/// <summary>Either a value or the error that stopped it.</summary>
public readonly struct Result<T, TError> : IEquatable<Result<T, TError>>
{
    private readonly T value;
    private readonly TError error;

    private Result(T value, TError error, bool isOk)
    {
        this.value = value;
        this.error = error;
        IsOk = isOk;
    }

    public bool IsOk { get; }

    public bool IsError => !IsOk;

    /// <summary>The value; the type's default when this is an error.</summary>
    public T Value => value;

    /// <summary>The error; the type's default when this succeeded.</summary>
    public TError Error => error;

    public static implicit operator Result<T, TError>(T value) => new(value, default!, true);

    public static implicit operator Result<T, TError>(Failure<TError> failure) => new(default!, failure.Error, false);

    /// <summary>The value when this succeeded, else the error.</summary>
    public bool TryGet([MaybeNullWhen(false)] out T value, [MaybeNullWhen(true)] out TError error)
    {
        value = this.value;
        error = this.error;
        return IsOk;
    }

    /// <summary>This result's error as a failure of another value type.</summary>
    public Failure<TError> AsFailure() => new(error);

    public Result<TOther, TError> Map<TOther>(Func<T, TOther> map) => IsOk ? map(value) : new Failure<TError>(error);

    public Result<T, TOther> MapError<TOther>(Func<TError, TOther> map) => IsOk ? value : new Failure<TOther>(map(error));

    public T ValueOr(T fallback) => IsOk ? value : fallback;

    public bool Equals(Result<T, TError> other) =>
        IsOk == other.IsOk && (IsOk ? EqualityComparer<T>.Default.Equals(value, other.value) : EqualityComparer<TError>.Default.Equals(error, other.error));

    public override bool Equals(object? obj) => obj is Result<T, TError> other && Equals(other);

    public override int GetHashCode() => IsOk ? HashCode.Combine(true, value) : HashCode.Combine(false, error);

    public static bool operator ==(Result<T, TError> left, Result<T, TError> right) => left.Equals(right);

    public static bool operator !=(Result<T, TError> left, Result<T, TError> right) => !left.Equals(right);

    public override string ToString() => IsOk ? $"Ok({value})" : $"Err({error})";
}

/// <summary>A value that may be missing, for value and reference types alike.</summary>
public readonly record struct Maybe<T>
{
    private readonly T value;

    public Maybe(T value)
    {
        this.value = value;
        HasValue = true;
    }

    public bool HasValue { get; }

    /// <summary>The value; the type's default when there is none.</summary>
    public T Value => value;

    public bool TryGet([MaybeNullWhen(false)] out T value)
    {
        value = this.value;
        return HasValue;
    }

    public override string ToString() => HasValue ? $"Some({value})" : "None";
}

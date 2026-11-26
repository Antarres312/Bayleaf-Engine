using System.Globalization;
using System.Security.Cryptography;

namespace Bayleaf.Core;

/// <summary>
/// Stable 128-bit id for assets and scene objects. It stays the same across renames and moves. Files
/// store it as 32 lowercase hex digits. Its hash is seeded per process (<see cref="HashCode"/>), so ids
/// written into a file cannot be chosen to collide in the engine's maps.
/// </summary>
public readonly record struct StableId : IComparable<StableId>
{
    private readonly UInt128 value;

    private StableId(UInt128 value) => this.value = value;

    public static StableId FromUInt128(UInt128 value) => new(value);

    public UInt128 AsUInt128() => value;

    /// <summary>A new random id from the system random source.</summary>
    public static StableId Random()
    {
        Span<byte> bytes = stackalloc byte[16];
        RandomNumberGenerator.Fill(bytes);
        return new(System.Buffers.Binary.BinaryPrimitives.ReadUInt128LittleEndian(bytes));
    }

    /// <summary>Parses the 32-digit hex form written by <see cref="ToHex"/>.</summary>
    public static Result<StableId, string> ParseHex(string text)
    {
        if (text.Length != 32 || !text.All(char.IsAsciiHexDigit))
        {
            return Result.Fail($"invalid id: {text}");
        }
        return UInt128.TryParse(text, NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out var parsed) ? new StableId(parsed) : Result.Fail($"invalid id: {text}");
    }

    public string ToHex() => value.ToString("x32", CultureInfo.InvariantCulture);

    public override string ToString() => ToHex();

    public bool Equals(StableId other) => value == other.value;

    public override int GetHashCode() => HashCode.Combine((ulong)value, (ulong)(value >> 64));

    public int CompareTo(StableId other) => value.CompareTo(other.value);

    public static bool operator <(StableId left, StableId right) => left.CompareTo(right) < 0;

    public static bool operator >(StableId left, StableId right) => left.CompareTo(right) > 0;

    public static bool operator <=(StableId left, StableId right) => left.CompareTo(right) <= 0;

    public static bool operator >=(StableId left, StableId right) => left.CompareTo(right) >= 0;
}

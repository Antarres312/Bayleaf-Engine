namespace Bayleaf.Core.Tests;

public class StableIdTests
{
    [Fact]
    public void HexRoundTripKeepsTheId()
    {
        var id = StableId.FromUInt128(new UInt128(0x0123_4567_89ab_cdef, 0x0011_2233_4455_6677));
        Assert.Equal(id, StableId.ParseHex(id.ToHex()).Value);
    }

    [Fact]
    public void ParseHexRejectsWrongLengthSignsAndNonHex()
    {
        Assert.True(StableId.ParseHex("abc").IsError);
        Assert.True(StableId.ParseHex(new string('z', 32)).IsError);
        Assert.True(StableId.ParseHex("+" + new string('0', 31)).IsError);
        Assert.True(StableId.ParseHex(" " + new string('0', 31)).IsError);
    }

    [Fact]
    public void RandomIdsDiffer() => Assert.NotEqual(StableId.Random(), StableId.Random());
}

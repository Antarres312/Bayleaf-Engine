using Bayleaf.Core.Json;

namespace Bayleaf.Core.Tests;

public class JsonTests
{
    private static JsonValue Parse(string text) => JsonReader.Parse(text).Value;

    [Fact]
    public void WholeNumbersStayWholeAndEverythingElseIsADouble()
    {
        var items = Parse("[1, -1, 1.0, 1e2, -0, 18446744073709551615, 18446744073709551616]").AsArray()!;
        var kinds = items.Select(i => ((JsonNumber)i).NumberKind).ToList();
        Assert.Equal(
            [NumberKind.Natural, NumberKind.NegativeWhole, NumberKind.Real, NumberKind.Real, NumberKind.Real, NumberKind.Natural, NumberKind.Real],
            kinds);
        Assert.True(double.IsNegative(((JsonNumber)items[4]).ToDouble()), "-0 reads as negative zero");
        Assert.Equal("[1,-1,1.0,100.0,-0.0,18446744073709551615,1.8446744073709552e+19]", JsonWriter.Compact(items));
    }

    [Fact]
    public void ShortestDigitsArePlainNearOneAndExponentialFarFromIt()
    {
        Assert.Equal("0.00001", JsonNumbers.Format(1e-5));
        Assert.Equal("1e-6", JsonNumbers.Format(1e-6));
        Assert.Equal("1000000000000000.0", JsonNumbers.Format(1e15));
        Assert.Equal("1e+16", JsonNumbers.Format(1e16));
        Assert.Equal("1.25e+20", JsonNumbers.Format(1.25e20));
        // Single precision has its own range and digits; widened to double it shows its error.
        Assert.Equal("0.000001", JsonNumbers.Format(1e-6f));
        Assert.Equal("1e-7", JsonNumbers.Format(1e-7f));
        Assert.Equal("0.8", JsonNumbers.Format(0.8f));
        Assert.Equal("0.800000011920929", JsonNumbers.Format((double)0.8f));
    }

    [Fact]
    public void TextTheRustEngineRefusesIsRefused()
    {
        foreach (var text in new[] { "[1,]", "{\"a\": 1,}", "01", "1.", "-", "\"a\u0001\"", "[1] x", "\"\\ud800\"", "1e400", "\ufeff{}", "{'a': 1}", "// x\n1" })
        {
            Assert.True(JsonReader.Parse(text).IsError, $"accepted {text}");
        }
        Assert.True(JsonReader.Parse(new string('[', 128) + new string(']', 128)).IsOk, "128 levels are allowed");
        Assert.True(JsonReader.Parse(new string('[', 129) + new string(']', 129)).IsError, "129 are not");
    }

    [Fact]
    public void StringsEscapeOnlyQuotesBackslashesAndControlCharacters()
    {
        var text = Parse("\"Ł \\u00e9 \\\"q\\\" \\\\ / \\t \\u0001 \\ud83d\\ude00\"");
        Assert.Equal("\"Ł é \\\"q\\\" \\\\ / \\t \\u0001 😀\"", text.ToCompact());
    }

    [Fact]
    public void UntypedValuesSortTheirKeysByUtf8AndKeepTheLastDuplicate()
    {
        var value = Parse("{\"b\": 1, \"😀\": 2, \"～\": 3, \"a\": 4, \"b\": 5}").Canonical();
        Assert.Equal("{\"a\":4,\"b\":5,\"～\":3,\"😀\":2}", value.ToCompact());
        Assert.Equal(Parse("{\"a\": 4, \"b\": 5, \"～\": 3, \"😀\": 2}"), value);
        Assert.NotEqual(Parse("1"), Parse("1.0"));
    }

    [Fact]
    public void PrettyTextIndentsByTwoSpacesAndKeepsEmptyListsShort()
    {
        Assert.Equal("{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    1,\n    {\n      \"d\": null\n    }\n  ]\n}", Parse("{\"a\":[],\"b\":{},\"c\":[1,{\"d\":null}]}").ToPretty());
    }
}

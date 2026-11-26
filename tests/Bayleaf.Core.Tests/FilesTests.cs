using System.Text;

namespace Bayleaf.Core.Tests;

public class FilesTests
{
    [Fact]
    public void OnADesktopAMountedPackComesBeforeTheDiskAndWritesGoToTheDisk()
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-files-pack-{Environment.ProcessId}");
        if (Directory.Exists(dir))
        {
            Directory.Delete(dir, true);
        }
        Directory.CreateDirectory(Path.Join(dir, "saves"));
        File.WriteAllText(Path.Join(dir, "saves", "progress.json"), "on disk");
        try
        {
            Files.Mount(Path.Join(dir, "game.bproject"), Encoding.UTF8.GetBytes("packed"));
            Assert.Equal("packed", Encoding.UTF8.GetString(Files.Read(Path.Join(dir, "game.bproject")).Value));
            Assert.False(File.Exists(Path.Join(dir, "game.bproject")));
            Assert.True(Files.IsFile(Path.Join(dir, "game.bproject")));
            Assert.Equal("on disk", Encoding.UTF8.GetString(Files.Read(Path.Join(dir, "saves", "progress.json")).Value));
            var listed = Files.ReadDirectory(dir).Value.Order(StringComparer.Ordinal).ToList();
            Assert.Equal([Files.Plain(Path.Join(dir, "game.bproject")), Files.Plain(Path.Join(dir, "saves"))], listed.Select(Files.Plain));
            Assert.True(Files.Write(Path.Join(dir, "saves", "new.json"), "kept"u8).IsOk);
            Assert.True(File.Exists(Path.Join(dir, "saves", "new.json")));
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }

    [Fact]
    public void TheStoreFindsAFileByAnySpellingOfItsPathAndListsItsFolders()
    {
        var store = new FileStore();
        store.Mount("/game/scenes/main.bscene", "{}"u8.ToArray());
        store.Mount("/game/scenes/deep/other.bscene", []);
        store.Mount("/game/game.bproject", []);
        Assert.Equal("{}"u8.ToArray(), store.Read("/game/./materials/../scenes/main.bscene").Value);
        Assert.True(store.IsDirectory("/game/scenes"));
        Assert.False(store.IsFile("/game/scenes"));
        Assert.False(store.IsDirectory("/game/game.bproject"));
        var listed = store.ReadDirectory("/game").Value.Order(StringComparer.Ordinal).ToList();
        Assert.Equal([Files.Plain("/game/game.bproject"), Files.Plain("/game/scenes")], listed);
        Assert.True(store.Read("/game/missing").IsError);
    }

    [Fact]
    public void WritesReplaceTheFileAreReportedOnceAndRemovingForgetsIt()
    {
        var store = new FileStore();
        const string saves = "/game/saves/progress.json";
        store.Write(saves, "1"u8.ToArray());
        store.Write(saves, "2"u8.ToArray());
        Assert.Equal("2"u8.ToArray(), store.Read(saves).Value);
        Assert.Equal([Files.Plain(saves)], store.TakeWritten());
        Assert.Empty(store.TakeWritten());
        Assert.Equal(1, store.Length(saves).Value);
        Assert.True(store.RemoveFile(saves).IsOk);
        Assert.True(store.Read(saves).IsError);
        Assert.True(store.RemoveFile(saves).IsError);
    }
}

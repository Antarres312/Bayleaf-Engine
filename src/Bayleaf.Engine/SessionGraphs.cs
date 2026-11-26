using Bayleaf.Render;

using GraphModel = Bayleaf.Graph.Graph;

namespace Bayleaf.Engine;

/// <summary>The graph open in the graph editor: material graphs, visual scripts and animation states.</summary>
public sealed class GraphDoc
{
    internal GraphDoc(GraphModel graph, string? path)
    {
        Graph = graph;
        Path = path;
    }

    /// <summary>Which open graph it is, for as long as it is open.</summary>
    public ulong Id { get; internal set; }

    public GraphModel Graph { get; internal set; }

    /// <summary>File the graph was opened from or last saved to.</summary>
    public string? Path { get; internal set; }

    /// <summary>True when the graph has changes that are not saved.</summary>
    public bool Dirty { get; internal set; }

    public Bayleaf.Graph.Catalog? Catalog => GraphCommands.CatalogForDomain(Graph.Domain);
}

/// <summary>The open graphs: one in use, which the <c>graph.*</c> commands work on, and the others, each in a tab of its own.</summary>
public sealed partial class Session
{
    private readonly List<GraphDoc> otherGraphs = [];

    /// <summary>The id the last graph opened got.</summary>
    private ulong lastGraphId;

    /// <summary>The graph the <c>graph.*</c> commands work on: the open graph last used.</summary>
    public GraphDoc? Graph { get; private set; }

    /// <summary>Every open graph, by id (the order they were opened in).</summary>
    public IReadOnlyList<GraphDoc> Graphs => (Graph is null ? otherGraphs : otherGraphs.Append(Graph)).OrderBy(g => g.Id).ToList();

    /// <summary>The open graph with <paramref name="id"/>.</summary>
    public GraphDoc? GraphById(ulong id) => Graph?.Id == id ? Graph : otherGraphs.Find(g => g.Id == id);

    /// <summary>Makes the open graph <paramref name="id"/> the one the <c>graph.*</c> commands work on.</summary>
    internal void ActivateGraph(ulong id)
    {
        if (Graph?.Id == id)
        {
            return;
        }
        var at = otherGraphs.FindIndex(g => g.Id == id);
        if (at < 0)
        {
            throw Stop.BadParams($"no open graph has id {id}");
        }
        var chosen = otherGraphs[at];
        otherGraphs.RemoveAt(at);
        if (Graph is not null)
        {
            otherGraphs.Add(Graph);
        }
        Graph = chosen;
    }

    /// <summary>Opens <paramref name="doc"/> as a new graph, the one the commands work on; gives its id.</summary>
    internal ulong AddGraph(GraphDoc doc)
    {
        lastGraphId++;
        doc.Id = lastGraphId;
        if (Graph is not null)
        {
            otherGraphs.Add(Graph);
        }
        Graph = doc;
        return lastGraphId;
    }

    /// <summary>Closes the graph the commands work on, with its undo steps; the one opened last before it takes its place.</summary>
    internal void CloseGraph()
    {
        if (Graph is not { } closed)
        {
            return;
        }
        Undo.RemoveAll(s => s.Before.Kind == SnapshotKind.Graph && s.Before.Graph == closed.Id);
        Redo.RemoveAll(s => s.Before.Kind == SnapshotKind.Graph && s.Before.Graph == closed.Id);
        Graph = null;
        if (otherGraphs.Count > 0)
        {
            var last = otherGraphs.MaxBy(g => g.Id)!;
            otherGraphs.Remove(last);
            Graph = last;
        }
    }

    /// <summary>The graph in use as an undo step keeps it; null without one.</summary>
    internal Snapshot? GraphSnapshot() => Graph is null ? null : new(SnapshotKind.Graph, Graph.Graph.ToValue().ToCompact(), Graph.Id);

    /// <summary>Puts a graph's step back in its own graph, wherever its tab is (it becomes the one in use); gives what it replaced.</summary>
    private Snapshot RestoreGraph(Snapshot snapshot)
    {
        const string closed = "the graph of this step is closed";
        try
        {
            ActivateGraph(snapshot.Graph);
        }
        catch (CommandException)
        {
            throw Stop.Failed(closed);
        }
        var open = Graph ?? throw Stop.Failed(closed);
        var graph = GraphCommands.FromSnapshot(snapshot, open.Catalog);
        if (graph.IsError)
        {
            throw Stop.Failed($"the undo step cannot be read: {graph.Error}");
        }
        var current = GraphSnapshot()!;
        open.Graph = graph.Value;
        open.Dirty = true;
        return current;
    }

    /// <summary>The open material graphs that have a file, to draw instead of it while they are edited.</summary>
    public IReadOnlyList<(string Path, GraphModel Graph)> LiveMaterials() =>
        (Graph is null ? otherGraphs : otherGraphs.Prepend(Graph))
            .Where(d => d.Graph.Domain == Material.Catalog.Domain && d.Path is not null)
            .Select(d => (d.Path!, d.Graph))
            .ToList();

    /// <summary>A material library that shows the open material graphs as they are, before they are saved.</summary>
    public MaterialLibrary MaterialLibrary()
    {
        var library = new MaterialLibrary();
        library.SetLives(LiveMaterials());
        return library;
    }
}

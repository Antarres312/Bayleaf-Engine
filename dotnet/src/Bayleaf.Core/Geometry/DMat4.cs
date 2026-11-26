namespace Bayleaf.Core.Geometry;

/// <summary>
/// A 4×4 matrix in double precision, kept as four columns; a point is transformed as <c>M · p</c>, so
/// in <c>A · B</c> the transform B applies first.
/// </summary>
public readonly record struct DMat4(Double4 XAxis, Double4 YAxis, Double4 ZAxis, Double4 WAxis)
{
    public static readonly DMat4 Identity = new(new(1, 0, 0, 0), new(0, 1, 0, 0), new(0, 0, 1, 0), new(0, 0, 0, 1));

    /// <summary>The entry at <paramref name="row"/>, <paramref name="column"/>.</summary>
    public double this[int row, int column] => Column(column)[row];

    public Double4 Column(int index) => index switch
    {
        0 => XAxis,
        1 => YAxis,
        2 => ZAxis,
        _ => WAxis,
    };

    private Double4 Apply(Double4 v) => XAxis * v.X + YAxis * v.Y + ZAxis * v.Z + WAxis * v.W;

    public static DMat4 operator *(DMat4 a, DMat4 b) => new(a.Apply(b.XAxis), a.Apply(b.YAxis), a.Apply(b.ZAxis), a.Apply(b.WAxis));

    /// <summary>Scales, then rotates, then moves.</summary>
    public static DMat4 FromScaleRotationTranslation(Double3 scale, DQuat rotation, Double3 translation)
    {
        Double4 Axis(int column, double s) => new(rotation.Entry(0, column) * s, rotation.Entry(1, column) * s, rotation.Entry(2, column) * s, 0);
        return new(Axis(0, scale.X), Axis(1, scale.Y), Axis(2, scale.Z), new(translation.X, translation.Y, translation.Z, 1));
    }

    /// <summary>The scale, rotation and translation of an affine matrix without shear; a mirror shows as a negative X scale.</summary>
    public (Double3 Scale, DQuat Rotation, Double3 Translation) ToScaleRotationTranslation()
    {
        var sign = Determinant() < 0 ? -1.0 : 1.0;
        var scale = new Double3(XAxis.Xyz.Length * sign, YAxis.Xyz.Length, ZAxis.Xyz.Length);
        var columns = new[] { XAxis.Xyz * (1 / scale.X), YAxis.Xyz * (1 / scale.Y), ZAxis.Xyz * (1 / scale.Z) };
        var rotation = DQuat.FromRotationMatrix((row, column) => row switch
        {
            0 => columns[column].X,
            1 => columns[column].Y,
            _ => columns[column].Z,
        });
        return (scale, rotation, WAxis.Xyz);
    }

    public double Determinant()
    {
        var det = 0.0;
        for (var column = 0; column < 4; column++)
        {
            det += this[0, column] * Cofactor(this, 0, column);
        }
        return det;
    }

    /// <summary>The inverse, by cofactors; not finite for a matrix with no inverse.</summary>
    public DMat4 Inverse()
    {
        var m = this;
        var det = Determinant();
        var inv = new double[4, 4];
        for (var row = 0; row < 4; row++)
        {
            for (var column = 0; column < 4; column++)
            {
                // The inverse's (row, column) is the cofactor of (column, row) over the determinant.
                inv[row, column] = Cofactor(m, column, row) / det;
            }
        }
        Double4 Col(int c) => new(inv[0, c], inv[1, c], inv[2, c], inv[3, c]);
        return new(Col(0), Col(1), Col(2), Col(3));
    }

    private static double Cofactor(DMat4 m, int skipRow, int skipColumn)
    {
        var rows = Enumerable.Range(0, 4).Where(i => i != skipRow).ToArray();
        var cols = Enumerable.Range(0, 4).Where(i => i != skipColumn).ToArray();
        var minor = m[rows[0], cols[0]] * (m[rows[1], cols[1]] * m[rows[2], cols[2]] - m[rows[1], cols[2]] * m[rows[2], cols[1]])
            - m[rows[0], cols[1]] * (m[rows[1], cols[0]] * m[rows[2], cols[2]] - m[rows[1], cols[2]] * m[rows[2], cols[0]])
            + m[rows[0], cols[2]] * (m[rows[1], cols[0]] * m[rows[2], cols[1]] - m[rows[1], cols[1]] * m[rows[2], cols[0]]);
        return (skipRow + skipColumn) % 2 == 0 ? minor : -minor;
    }

    /// <summary>Whether every entry is within <paramref name="tolerance"/> of the other matrix's.</summary>
    public bool NearlyEquals(DMat4 other, double tolerance)
    {
        for (var row = 0; row < 4; row++)
        {
            for (var column = 0; column < 4; column++)
            {
                if (Math.Abs(this[row, column] - other[row, column]) > tolerance)
                {
                    return false;
                }
            }
        }
        return true;
    }
}

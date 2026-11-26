# Languages 2026-11-24

Median over runs of the mean milliseconds per step; in brackets, times C++ with objects owning their components.

## Script step

| Version | 5000 objects | 10000 objects | Runs' means, min-max |
|---|---:|---:|---|
| C++ (MSVC /O2), objects own components | 0.059 (1.0) | 0.117 (1.0) | 0.058-0.059; 0.116-0.117 |
| C++ (MSVC /O2), packed arrays | 0.040 (0.7) | 0.081 (0.7) | 0.040-0.040; 0.080-0.081 |
| C# (.NET Framework 4.8), objects own components | 0.272 (4.6) | 0.545 (4.7) | 0.270-0.272; 0.543-0.546 |
| C# (.NET Framework 4.8), packed arrays | 0.263 (4.5) | 0.523 (4.5) | 0.262-0.263; 0.523-0.524 |
| C# (.NET 9.0.22), objects own components | 0.099 (1.7) | 0.183 (1.6) | 0.098-0.099; 0.180-0.186 |
| C# (.NET 9.0.22), packed arrays | 0.125 (2.1) | 0.249 (2.1) | 0.125-0.125; 0.249-0.250 |
| Python (3.13.17), objects own components | 2.216 (38) | 5.830 (50) | 2.199-2.309; 5.668-6.160 |

## Script step and world matrices

| Version | 5000 objects | 10000 objects | Runs' means, min-max |
|---|---:|---:|---|
| C++ (MSVC /O2), objects own components | 0.125 (1.0) | 0.251 (1.0) | 0.124-0.126; 0.249-0.253 |
| C++ (MSVC /O2), packed arrays | 0.119 (1.0) | 0.235 (0.9) | 0.117-0.119; 0.234-0.235 |
| C# (.NET Framework 4.8), objects own components | 1.015 (8.1) | 2.044 (8.1) | 1.013-1.024; 2.031-2.069 |
| C# (.NET Framework 4.8), packed arrays | 1.015 (8.1) | 2.024 (8.1) | 1.011-1.018; 2.023-2.025 |
| C# (.NET 9.0.22), objects own components | 0.209 (1.7) | 0.429 (1.7) | 0.209-0.216; 0.428-0.462 |
| C# (.NET 9.0.22), packed arrays | 0.248 (2.0) | 0.498 (2.0) | 0.247-0.249; 0.495-0.501 |
| Python (3.13.17), objects own components | 8.172 (65) | 21.168 (84) | 6.806-9.278; 21.043-21.600 |


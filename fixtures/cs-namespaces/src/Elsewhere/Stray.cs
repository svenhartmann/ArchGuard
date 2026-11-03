using System;

namespace Cyan;

// Deliberate fixture flaw for the golden test: the file lives under
// src/Elsewhere/ but declares itself part of the Cyan namespace.
public class Stray
{
    public int Odd => 7;
}

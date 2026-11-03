<?php

namespace App\Two;

use App\One\One;

class Two
{
    public function name(): string
    {
        return 'two+' . (new One())->describe();
    }
}

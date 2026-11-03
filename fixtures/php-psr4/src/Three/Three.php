<?php

namespace App\Three;

use App\One\One;

class Three
{
    public function mix(): string
    {
        return 'three+' . (new One())->describe();
    }
}

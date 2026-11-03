<?php

namespace App;

use App\Three\Three;

class Boot
{
    public function start(): string
    {
        return (new Three())->mix();
    }
}

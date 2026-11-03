<?php

namespace App\One;

use App\Two\Two;

class One
{
    public function describe(): string
    {
        return 'one+' . (new Two())->name();
    }
}

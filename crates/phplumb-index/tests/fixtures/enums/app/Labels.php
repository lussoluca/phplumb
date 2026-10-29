<?php

namespace App;

trait Labels
{
    public function lowLabel(): string
    {
        return self::Low->label();
    }
}

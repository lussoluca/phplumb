<?php

namespace App;

/** @template V */
class Pack
{
    public function filter(): static
    {
        return $this;
    }

    public static function make(): static
    {
        return new static();
    }

    public function /*@target:packed*/size(): int
    {
        return 0;
    }
}

class Packer
{
    /** @param Pack<Item> $pack */
    public function run(Pack $pack): void
    {
        $x = $pack->filter();
        $x::make()->/*@cursor:packed*/size();
    }
}

<?php

namespace App;

class Item
{
    public function /*@target:cond*/name(): string
    {
        return '';
    }
}

class Fallback
{
    public function /*@target:cond*/name(): string
    {
        return '';
    }
}

class Other
{
    public function name(): string
    {
        return '';
    }
}

/** @template T */
class Holder
{
    /** @return (T is null ? Fallback : Item) */
    public function pick()
    {
        return null;
    }
}

class UsesHolder
{
    /** @param Holder<Other> $holder */
    public function run(Holder $holder): void
    {
        $holder->pick()->/*@cursor:cond*/name();
    }
}

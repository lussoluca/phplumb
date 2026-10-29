<?php

namespace App;

class Item
{
    public function /*@target:static*/label(): string
    {
        return '';
    }
}

/**
 * @template K
 * @template V
 */
class Bag
{
    public function filter(): static
    {
        return $this;
    }

    /** @return V */
    public function first()
    {
        return null;
    }

    public static function make(): static
    {
        return new static();
    }

    public function inner(): void
    {
        self::make()->first()->/*@cursor:forwarded*/label();
    }
}

class Shelf
{
    /** @param Bag<int, Item> $bag */
    public function use(Bag $bag): void
    {
        $bag->filter()->first()->/*@cursor:static*/label();
    }
}

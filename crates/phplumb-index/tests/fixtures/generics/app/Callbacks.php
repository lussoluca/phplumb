<?php

namespace App;

class Fruit
{
    public function /*@target:callback*/name(): string
    {
        return '';
    }
}

/** @template T */
class Items
{
    /**
     * @param Closure(T): bool $p
     * @return static
     */
    public function filter(\Closure $p)
    {
        return $this;
    }
}

class CallbackUses
{
    /** @param Items<Fruit> $items */
    public function run(Items $items): void
    {
        $items->filter(fn ($i) => $i->/*@cursor:callback*/name());
        $items->filter(fn ($i, $k) => $k->/*@cursor:extra*/name());
    }
}

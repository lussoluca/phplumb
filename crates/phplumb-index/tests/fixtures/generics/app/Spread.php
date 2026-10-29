<?php

namespace App;

class Leaf
{
    public function /*@target:spread*/name(): string
    {
        return '';
    }
}

class SpreadUses
{
    /**
     * @param list<list<Leaf>> $lists
     * @param list<Leaf> $flat
     */
    public function run(array $lists, array $flat): void
    {
        array_map(fn ($x) => $x->/*@cursor:spread*/name(), ...$lists);
        array_map(fn ($y) => $y->/*@cursor:flat*/name(), ...$flat);
    }
}

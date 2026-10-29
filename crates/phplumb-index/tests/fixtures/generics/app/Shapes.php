<?php

namespace App;

class Clock
{
    public function /*@target:shape*/format(): string
    {
        return '';
    }
}

class Left
{
    public function /*@target:left*/name(): string
    {
        return '';
    }
}

class Right
{
    public function /*@target:right*/size(): int
    {
        return 0;
    }
}

class ShapeUses
{
    /**
     * @param array{startedAt: Clock, id?: int} $record
     * @param array{Left, Right} $pair
     * @param list<array{Left, Right}> $rows
     * @param array{first: Left} $named
     */
    public function run(array $record, array $pair, array $rows, array $named, string $key): void
    {
        $record['startedAt']->/*@cursor:shape*/format();
        $record[$key]->/*@cursor:dynamic*/format();
        $record['missing']->/*@cursor:missing*/format();
        [$a, $b] = $pair;
        $a->/*@cursor:left*/name();
        $b->/*@cursor:right*/size();
        foreach ($rows as [$c, $d]) {
            $c->/*@cursor:left*/name();
            $d->/*@cursor:right*/size();
        }
        ['first' => $e] = $named;
        $e->/*@cursor:left*/name();
    }
}

<?php

namespace App;

class Entry
{
    public function /*@target:inherited*/label(): string
    {
        return '';
    }
}

class Other
{
    public function /*@target:own*/name(): string
    {
        return '';
    }
}

interface Handler
{
    /** @param list<Entry> $items */
    public function handle(array $items): void;
}

class ListHandler implements Handler
{
    public function handle(array $items): void
    {
        foreach ($items as $item) {
            $item->/*@cursor:inherited*/label();
        }
    }
}

class OwnHandler implements Handler
{
    /** @param Other $items */
    public function handle($items): void
    {
        $items->/*@cursor:own*/name();
    }
}

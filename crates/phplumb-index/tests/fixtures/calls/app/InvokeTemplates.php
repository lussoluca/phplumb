<?php

namespace App;

class Note
{
    public function /*@target:invoked*/label(): string
    {
        return '';
    }
}

/** @template T */
class Factory
{
    /**
     * @template T
     * @param class-string<T> $class
     * @return T
     */
    public function __invoke(string $class)
    {
        return new $class();
    }
}

class Maker
{
    /** @param Factory<Entry> $factory */
    public function run(Factory $factory): void
    {
        ($factory)(Note::class)->/*@cursor:invoked*/label();
    }
}

<?php

namespace App;

class Goods
{
    public function /*@target:parent*/ /*@target:trait*/name(): string
    {
        return '';
    }
}

/** @template T */
class Base
{
    /** @return T */
    public function get()
    {
        return null;
    }
}

/** @extends Base<Goods> */
class Sub extends Base
{
    public function run(): void
    {
        parent::get()->/*@cursor:parent*/name();
    }
}

/** @template T */
trait Holds
{
    /** @return T */
    public function held()
    {
        return null;
    }
}

/** @template T */
trait Keeps
{
    /** @return T */
    public function kept()
    {
        return null;
    }
}

class Shelf
{
    /** @use Holds<Goods> */
    use Holds;
    use Keeps;
}

class Plain
{
    public function /*@target:plain*/own(): void
    {
    }
}

class ShelfUses
{
    /** @param Plain<int, Goods> $plain */
    public function run(Shelf $shelf, Plain $plain): void
    {
        $shelf->held()->/*@cursor:trait*/name();
        $shelf->kept()->/*@cursor:leak*/name();
        $plain->/*@cursor:plain*/own();
    }
}

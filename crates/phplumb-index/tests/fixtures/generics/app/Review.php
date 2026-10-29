<?php

namespace App;

class Maker
{
    /** @return array{self, int} */
    public function pair(): array
    {
        return [$this, 1];
    }

    public function /*@target:selfshape*/build(): void
    {
    }
}

class Caller
{
    public function build(): void
    {
    }

    public function run(Maker $m): void
    {
        [$a, $n] = $m->pair();
        $a->/*@cursor:selfshape*/build();
    }
}

/** @template T */
class Crate
{
    /** @param \Closure(static): void $f */
    public function tap(\Closure $f): void
    {
    }

    public function /*@target:staticcb*/ping(): void
    {
    }
}

class CrateUser
{
    public function ping(): void
    {
    }

    public function run(Crate $b): void
    {
        $b->tap(fn ($x) => $x->/*@cursor:staticcb*/ping());
    }
}

class U
{
    public function name(): string
    {
        return '';
    }
}

/** @template T */
class Tier
{
    /** @return T */
    public function get()
    {
        return null;
    }
}

/**
 * @template U
 * @extends Tier<U>
 */
class Mid extends Tier
{
    public function run(): void
    {
        parent::get()->/*@cursor:ptpl*/name();
    }
}

class Thing
{
    public function /*@target:calldoc*/ /*@target:above*/name(): string
    {
        return '';
    }
}

/**
 * @template K
 * @template V
 */
class Coll
{
    /** @return V */
    public function first()
    {
        return null;
    }
}

class Gadget
{
    public const KIND = 'k';

    public function /*@target:colon*/ /*@target:constb*/run(): void
    {
    }
}

class Parented
{
    public function make(): Thing
    {
        return new Thing();
    }
}

class Childed extends Parented
{
    /** @return callable(int): void */
    public function make()
    {
        return new Thing();
    }
}

class MixUser
{
    /**
     * @param Coll<int, mixed> $c
     * @param Gadget: $g
     * @param array{Gadget::KIND, Gadget} $p
     */
    public function run(Coll $c, Thing $t, bool $f, $g, array $p, Childed $child): void
    {
        $v = $t;
        if ($f) {
            $v = $c->first();
        }
        $v->/*@cursor:mixed*/name();
        /** @var callable(int): void $fn */
        $fn = new Thing();
        $fn->/*@cursor:calldoc*/name();
        $g->/*@cursor:colon*/run();
        [$k, $h] = $p;
        $k->/*@cursor:const*/run();
        $h->/*@cursor:constb*/run();
        $child->make()->/*@cursor:above*/name();
    }
}

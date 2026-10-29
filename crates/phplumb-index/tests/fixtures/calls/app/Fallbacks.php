<?php

namespace App;

class Service
{
    public function label(): string
    {
        return '';
    }
}

interface Visitor
{
    /** @param Entry $entry */
    public function visit($entry): void;
}

class Walker implements Visitor
{
    public function visit($entry): void
    {
        $list = [];
        $f = function ($x) {
            $x->/*@cursor:closure*/label();
        };
        $g = fn ($y) => $y->/*@cursor:arrow*/label();
        usort($list, function ($a, $b) {
            return $a->/*@cursor:usort*/label();
        });
    }
}

class Base
{
    public function __construct(Service $service)
    {
    }

    private function hidden(Service $service): void
    {
    }
}

class Child extends Base
{
    public function __construct($options)
    {
        $options->/*@cursor:ctor*/label();
    }

    public function hidden($value): void
    {
        $value->/*@cursor:private*/label();
    }
}

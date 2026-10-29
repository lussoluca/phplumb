<?php

namespace App;

enum Severity: string
{
    use Labels;

    case Low = 'low';
    case High = 'high';

    const LIMIT = self::High;

    public function label(): string
    {
        return ucfirst($this->value);
    }

    public function lowest(): string
    {
        return self::Low->label();
    }

    public function lowestLate(): string
    {
        return static::Low->label();
    }

    public function lowestThis(): string
    {
        return $this::Low->label();
    }
}

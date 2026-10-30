<?php

namespace Symfony\Component\Messenger;

final class Envelope
{
    public function __construct(object $message, array $stamps = [])
    {
    }

    public static function wrap(object $message, array $stamps = []): self
    {
        return new self($message, $stamps);
    }
}

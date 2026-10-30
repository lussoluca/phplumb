<?php

namespace Symfony\Component\Messenger;

trait HandleTrait
{
    private function handle(object $message, array $stamps = []): mixed
    {
        return null;
    }
}

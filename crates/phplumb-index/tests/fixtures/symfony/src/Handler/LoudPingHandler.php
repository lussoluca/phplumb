<?php

namespace App\Handler;

use App\Message\LoudPing;
use Symfony\Component\Messenger\Attribute\AsMessageHandler;

#[AsMessageHandler]
final class LoudPingHandler
{
    public function __invoke(LoudPing $message): void
    {
    }
}

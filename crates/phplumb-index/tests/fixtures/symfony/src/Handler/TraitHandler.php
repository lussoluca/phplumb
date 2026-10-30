<?php

namespace App\Handler;

use Symfony\Component\Messenger\Attribute\AsMessageHandler;

#[AsMessageHandler]
final class TraitHandler
{
    use HandlesEmail;
}

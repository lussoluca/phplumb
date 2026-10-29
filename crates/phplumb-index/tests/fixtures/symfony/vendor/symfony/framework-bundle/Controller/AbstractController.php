<?php

namespace Symfony\Bundle\FrameworkBundle\Controller;

abstract class AbstractController
{
    protected function generateUrl(string $route, array $parameters = []): string
    {
        return '';
    }

    protected function render(string $view, array $parameters = []): object
    {
        return new \stdClass();
    }

    protected function renderView(string $view, array $parameters = []): string
    {
        return '';
    }

    protected function renderBlock(string $view, string $block, array $parameters = []): object
    {
        return new \stdClass();
    }

    protected function renderBlockView(string $view, string $block, array $parameters = []): string
    {
        return '';
    }

    protected function stream(string $view, array $parameters = []): object
    {
        return new \stdClass();
    }

    protected function getParameter(string $name): mixed
    {
        return null;
    }

    protected function redirectToRoute(string $route, array $parameters = []): object
    {
        return new \stdClass();
    }
}
